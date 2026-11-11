//! The scheduler (`docs/architecture.md`, 10.4): the ready nodes by their position, the sleepers by their time, the keys of
//! native calls, the frames of calls and the instances of regions, and how a run winds down after an abort or a fault.

use std::any::Any;
use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashMap};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use labcraft_ir::{FunctionId, Node, Program, Region, Skip};
use labcraft_types::{Interner, Name, Ty};

use super::memory::{Memory, NODE_COST};
use super::plan::Plan;
use super::position::{Pos, child, compare, root};
use super::{
    AbortAt, AbortBy, Applied, CLOCK_END, Control, FaultAt, Live, LoopAt, Observer, Outputs, PanelSetting, Progress, RealTime, Resource, Run, RunEnd,
    SetBy, Settings, Status,
};
use crate::convert::{Env, constant, read};
use crate::fault::{Fault, FaultCode};
use crate::json::Describe;
use crate::native::{CallContext, CallId, HandleInfo, Job, Key, LibraryRun, LogBudget, NativeOutcome, Step};
use crate::pool::Work;
use crate::value::{ErrorData, Handle, Origin, Place, Value};

pub(super) type InstanceId = usize;
pub(super) type TaskId = usize;
pub(super) type FrameId = usize;

/// A fault where it happened.
pub(super) type Halt = Box<FaultAt>;

/// What the scheduler runs next.
#[derive(Clone, Copy, Debug)]
pub(super) enum Ready {
    /// A node whose inputs are all there.
    Start { instance: InstanceId, node: u32 },
    /// A node that waited: a native call goes on, a `time.wait` ends.
    Resume(TaskId),
}

#[derive(Debug)]
struct Ticket {
    pos: Pos,
    seq: u64,
    what: Ready,
}

impl PartialEq for Ticket {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Ticket {}

impl PartialOrd for Ticket {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ticket {
    /// The first in the global order is the greatest, for the heap.
    fn cmp(&self, other: &Self) -> Ordering {
        compare(&other.pos, &self.pos).then(other.seq.cmp(&self.seq))
    }
}

/// SplitMix64 (Steele, Lea and Flood, 2014): the draw of a schedule seed.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// The ready nodes: by position in a heap, or, with a schedule seed, in a list where one is drawn and swapped with the last.
enum ReadyQueue {
    Ordered(BinaryHeap<Ticket>),
    Drawn(Vec<Ticket>, SplitMix64),
}

impl ReadyQueue {
    fn push(&mut self, ticket: Ticket) {
        match self {
            ReadyQueue::Ordered(heap) => heap.push(ticket),
            ReadyQueue::Drawn(list, _) => list.push(ticket),
        }
    }

    fn pop(&mut self) -> Option<Ticket> {
        match self {
            ReadyQueue::Ordered(heap) => heap.pop(),
            ReadyQueue::Drawn(list, draw) => {
                if list.is_empty() {
                    return None;
                }
                let length = u64::try_from(list.len()).unwrap_or(u64::MAX);
                let index = usize::try_from(draw.next() % length).unwrap_or(0);
                Some(list.swap_remove(index))
            }
        }
    }

    fn drain(&mut self) -> Vec<Ticket> {
        match self {
            ReadyQueue::Ordered(heap) => std::mem::take(heap).into_vec(),
            ReadyQueue::Drawn(list, _) => std::mem::take(list),
        }
    }
}

/// The loops around an instance, innermost first, through the frames of calls.
#[derive(Debug)]
pub(super) struct LoopLink {
    pub(super) function: FunctionId,
    pub(super) node: Name,
    pub(super) iteration: u64,
    pub(super) parent: Option<Rc<LoopLink>>,
}

impl Drop for LoopLink {
    fn drop(&mut self) {
        let mut next = self.parent.take();
        while let Some(link) = next {
            match Rc::try_unwrap(link) {
                Ok(mut found) => next = found.parent.take(),
                Err(_) => break,
            }
        }
    }
}

/// A call waiting for a key, by its position: the first in the global order gets it.
struct Waiter {
    pos: Pos,
    seq: u64,
    task: TaskId,
}

impl PartialEq for Waiter {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Waiter {}

impl PartialOrd for Waiter {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Waiter {
    fn cmp(&self, other: &Self) -> Ordering {
        compare(&other.pos, &self.pos).then(other.seq.cmp(&self.seq))
    }
}

/// Who an instance gives its results to.
#[derive(Clone, Copy, Debug)]
pub(super) enum Parent {
    Top,
    Task(TaskId),
}

/// One run of a region: the values of its arguments and of the outputs of its nodes, and what each node still waits for.
pub(super) struct Instance<'p> {
    pub(super) region: &'p Region,
    pub(super) plan: Rc<Plan>,
    slots: Vec<Option<Value>>,
    left: Vec<u32>,
    missing: Vec<u32>,
    pending: usize,
    pub(super) frame: FrameId,
    pub(super) pos: Pos,
    pub(super) iter: Rc<[u64]>,
    pub(super) loops: Option<Rc<LoopLink>>,
    pub(super) parent: Parent,
    cost: u64,
    /// The first arguments are not counted: the elements of the arrays of the `in` clauses of a `for`, which count in them.
    free: usize,
}

/// One call of a function written in LabCraft.
pub(super) struct Frame {
    pub(super) function: FunctionId,
    pub(super) number: u64,
    pub(super) depth: u32,
    /// By element: what the panel of this call holds (spec 10).
    pub(super) panel: Vec<Option<Value>>,
    parent: Option<FrameId>,
    /// The node of the caller that made this call.
    site: Option<Place>,
    /// The callers of an error made in this frame, made once.
    callers: Option<Arc<[Place]>>,
}

/// A node that does not end when it starts.
pub(super) struct Task<'p> {
    pub(super) instance: InstanceId,
    pub(super) node: u32,
    pub(super) pos: Pos,
    pub(super) state: State<'p>,
}

pub(super) enum State<'p> {
    /// A call of a function written in LabCraft, which runs in its frame.
    Call {
        frame: FrameId,
    },
    /// The arm of a `match` or a `handle` runs.
    Arm,
    /// `time.wait` sleeps.
    Wait,
    Native(NativeCall<'p>),
    Loop(LoopState<'p>),
    For(ForState<'p>),
}

pub(super) struct NativeCall<'p> {
    pub(super) library: usize,
    pub(super) call: CallId,
    /// The number of the work it gave the pool, while that work goes on.
    pub(super) blocked: Option<u64>,
    pub(super) held: Vec<Key>,
    /// The inputs as the skip reads them, for the outputs of a call that fails (spec 7.2).
    pub(super) inputs: Vec<Value>,
    pub(super) skip: &'p Skip,
}

pub(super) struct LoopState<'p> {
    pub(super) region: &'p Region,
    pub(super) iteration: u64,
    pub(super) carries: usize,
    /// The values passed in, which each iteration reads.
    pub(super) passed: Vec<Value>,
    pub(super) collects: Vec<Vec<Value>>,
    pub(super) continue_on_error: bool,
}

pub(super) enum Clause {
    Each(Value),
    Range { low: i128, kind: labcraft_types::NumberKind },
}

pub(super) struct ForState<'p> {
    pub(super) region: &'p Region,
    pub(super) parallel: bool,
    /// Iterations started.
    pub(super) iteration: u64,
    /// Iterations ended.
    pub(super) ended: u64,
    pub(super) count: u64,
    pub(super) clauses: Vec<Clause>,
    /// The carries between two iterations, which the next one takes; never counted here, since an instance counts them.
    pub(super) carries: Vec<Value>,
    pub(super) carry_count: usize,
    pub(super) passed: Vec<Value>,
    /// By collect, by iteration: a parallel loop ends its iterations in any order.
    pub(super) collects: Vec<Vec<Option<Value>>>,
}

pub(super) struct NativeSlot {
    name: String,
    run: Box<dyn LibraryRun>,
}

/// The libraries of the run, which say what a handle is when the trace writes it. A panic of one is caught: the handle then
/// shows only its type.
pub(super) struct Described<'a>(pub(super) &'a [NativeSlot]);

impl Describe for Described<'_> {
    fn describe(&self, handle: Handle) -> Option<HandleInfo> {
        let slot = self.0.get(handle.library as usize)?;
        catch_unwind(AssertUnwindSafe(|| slot.run.describe(handle))).ok().flatten()
    }
}

struct Opened {
    handle: Handle,
    function: FunctionId,
    node: Option<Name>,
    origin: Option<labcraft_graph::Id>,
    released: bool,
    leaked: bool,
}

/// Work of the pool that ended: what it returned, or nothing when it panicked.
struct JobDone {
    task: TaskId,
    serial: u64,
    out: Option<Box<dyn Any + Send>>,
}

enum Ending {
    Finished(Vec<Value>),
    Aborted(AbortAt),
    Faulted(FaultAt),
}

pub(super) struct Machine<'p, 'o> {
    pub(super) program: &'p Program,
    pub(super) names: &'p Interner,
    pub(super) settings: Settings,
    control: &'o mut dyn Control,
    pub(super) observer: &'o mut dyn Observer,
    pub(super) now: i64,
    seq: u64,
    ready: ReadyQueue,
    sleepers: BinaryHeap<Reverse<(i64, u64, TaskId)>>,
    instances: Vec<Option<Instance<'p>>>,
    free_instances: Vec<InstanceId>,
    pub(super) tasks: Vec<Option<Task<'p>>>,
    free_tasks: Vec<TaskId>,
    pub(super) frames: Vec<Option<Frame>>,
    free_frames: Vec<FrameId>,
    next_frame: u64,
    plans: HashMap<*const Region, Rc<Plan>>,
    pub(super) memory: Memory,
    pub(super) nodes_run: u64,
    pub(super) natives: Vec<NativeSlot>,
    pub(super) native_of: HashMap<labcraft_types::LibraryId, usize>,
    holders: HashMap<Key, TaskId>,
    waiting: HashMap<Key, BinaryHeap<Waiter>>,
    opened: Vec<Opened>,
    opened_index: HashMap<Handle, usize>,
    /// The settings of `set`, by time, of which those before `next_setting` have applied.
    panel: Vec<Applied>,
    next_setting: usize,
    /// The settings that applied, in the order they applied: those of `set` and of `run.set_element`.
    applied: Vec<Applied>,
    /// The settings of `run.set_element` taken that never applied, a fault ending the run first.
    never: Vec<Applied>,
    /// In real time: the pool, what wakes the scheduler, and the time of the start.
    real_time: Option<RealTime>,
    /// The work of the pool that ended, which the scheduler takes when it wakes.
    jobs: Arc<Mutex<Vec<JobDone>>>,
    /// Calls whose work is on the pool.
    blocked: usize,
    serial: u64,
    done: Vec<InstanceId>,
    ending: Option<Ending>,
    stopping: bool,
    /// For each function: its name as reports show it, and the file it is in.
    shown: Vec<(Arc<str>, Arc<str>)>,
    files: &'p [labcraft_check::CheckedFile],
    inputs: Vec<Value>,
    pub(super) err_name: Option<Name>,
    pub(super) button: Option<Name>,
    /// A library that panicked as its run started.
    broken: Option<String>,
}

impl ForState<'_> {
    pub(super) fn carries_count(&self) -> usize {
        self.carry_count
    }
}

impl<'p, 'o> Machine<'p, 'o> {
    pub(super) fn new(run: Run<'p>, control: &'o mut dyn Control, observer: &'o mut dyn Observer) -> Self {
        let Run { program, names, files, natives, inputs, mut settings } = run;
        let real_time = settings.real_time.take();
        let budget = LogBudget::of_run(settings.max_memory);
        let mut native_of = HashMap::new();
        let mut slots = Vec::with_capacity(natives.len());
        let mut broken = None;
        // A handle carries the index of its library among those of the run, which is its place here: a library that panics
        // as it starts ends the run before anything runs.
        for (index, native) in natives.iter().enumerate() {
            let number = u32::try_from(index).unwrap_or(u32::MAX);
            match catch_unwind(AssertUnwindSafe(|| native.native.start_run(number, native.simulation, budget.clone()))) {
                Ok(run) => {
                    native_of.insert(native.library, slots.len());
                    slots.push(NativeSlot { name: native.native.name().to_owned(), run });
                }
                Err(_) => {
                    broken = Some(native.native.name().to_owned());
                    break;
                }
            }
        }
        let shown = program
            .functions
            .iter()
            .map(|function| {
                let file = files.get(function.file as usize).map_or("", |file| file.shown.as_str());
                (Arc::from(&*function.shown), Arc::from(file))
            })
            .collect();
        let mut panel: Vec<Applied> = std::mem::take(&mut settings.panel).into_iter().map(|setting| Applied { setting, applied: false }).collect();
        panel.sort_by_key(|applied| applied.setting.at);
        let ready = match settings.schedule_seed {
            Some(seed) => ReadyQueue::Drawn(Vec::new(), SplitMix64(seed)),
            None => ReadyQueue::Ordered(BinaryHeap::new()),
        };
        Machine {
            program,
            names,
            memory: Memory::new(settings.max_memory, settings.max_made),
            settings,
            control,
            observer,
            now: 0,
            seq: 0,
            ready,
            sleepers: BinaryHeap::new(),
            instances: Vec::new(),
            free_instances: Vec::new(),
            tasks: Vec::new(),
            free_tasks: Vec::new(),
            frames: Vec::new(),
            free_frames: Vec::new(),
            next_frame: 0,
            plans: HashMap::new(),
            nodes_run: 0,
            natives: slots,
            native_of,
            holders: HashMap::new(),
            waiting: HashMap::new(),
            opened: Vec::new(),
            opened_index: HashMap::new(),
            panel,
            next_setting: 0,
            applied: Vec::new(),
            never: Vec::new(),
            real_time,
            jobs: Arc::default(),
            blocked: 0,
            serial: 0,
            done: Vec::new(),
            ending: None,
            stopping: false,
            shown,
            files,
            inputs,
            err_name: names.find("err"),
            button: names.find("button"),
            broken,
        }
    }

    pub(super) fn env(&self) -> Env<'p> {
        Env { program: self.program, names: self.names }
    }

    // The run ------------------------------------------------------------------------------------------------------------

    pub(super) fn go(mut self) -> RunEnd {
        let result = self.start_top().and_then(|()| self.drain()).and_then(|()| self.schedule());
        if let Err(fault) = result {
            self.ending = Some(Ending::Faulted(*fault));
        }
        if !matches!(self.ending, Some(Ending::Finished(_))) {
            self.wind_down();
        }
        self.end()
    }

    fn start_top(&mut self) -> Result<(), Halt> {
        if let Some(library) = &self.broken {
            return Err(self.halt_at(None, panicked(library)));
        }
        if self.settings.timeout.is_some_and(|timeout| timeout > CLOCK_END) {
            let fault = Fault::new(FaultCode::Clock, "the timeout of the run is past the end of its clock, 2^62 ns (cli.md 6.2)");
            return Err(self.halt_at(None, fault));
        }
        let program = self.program;
        let top = FunctionId(0);
        let Some(function) = program.functions.first() else { return Err(self.halt_at(None, Fault::bug("a program without a function"))) };
        let labcraft_ir::Body::Region(region) = &function.body else {
            return Err(self.halt_at(None, Fault::bug("a run starts at a function written in LabCraft, and this one is `extern`")));
        };
        let inputs = std::mem::take(&mut self.inputs);
        if inputs.len() != function.inputs.len() {
            return Err(self.halt_at(None, Fault::bug("the inputs of a run, not those of its function")));
        }
        let frame = self.new_frame(top, None, None, 1, &inputs).map_err(|fault| self.halt_at(None, fault))?;
        let number = self.frame(frame).map_or(0, |frame| frame.number);
        self.observer.call(self.now, number, None, top, &inputs, &Described(&self.natives));
        self.new_instance(region, inputs, frame, root(), Rc::from([]), None, Parent::Top).map_err(|fault| self.halt_at(None, fault))?;
        Ok(())
    }

    /// Runs until the function ends, an abort, or a fault. In real time the clock is read between two nodes, and what is due
    /// by then (the settings, the timeout, the sleepers, the work of the pool, the settings of `run.set_element`) is taken
    /// before the next node; with no node ready, the scheduler waits until the next time it knows of, or until it is woken.
    fn schedule(&mut self) -> Result<(), Halt> {
        self.tick();
        self.at_this_time()?;
        loop {
            if self.ending.is_some() {
                return Ok(());
            }
            // Taken before what can wake it is looked at, so that a wake that comes in between ends the wait at once.
            let seen = self.real_time.as_ref().map(|real| real.wake.seen());
            // The clock first, so that an abort is dated when it is seen.
            self.tick();
            if let Some(by) = self.control.abort() {
                self.abort(by);
                return Ok(());
            }
            if seen.is_some() {
                self.take_live()?;
                self.jobs_done()?;
                self.at_this_time()?;
                if self.ending.is_some() {
                    return Ok(());
                }
                self.wake_sleepers();
            }
            if self.control.wants_progress() {
                let progress = Progress { clock: self.now, opened: self.opened.len(), loops: self.loops_going() };
                self.control.progress(progress);
            }
            if let Some(ticket) = self.ready.pop() {
                self.check_real_time(&ticket)?;
                self.run_ticket(ticket)?;
                self.drain()?;
                continue;
            }
            let wake = self.sleepers.peek().map(|Reverse((wake, _, _))| *wake);
            let next_setting = self.panel.get(self.next_setting).map(|applied| applied.setting.at);
            if let Some(seen) = seen {
                let until = [wake, next_setting, self.settings.timeout].into_iter().flatten().min();
                if until.is_none() && self.blocked == 0 {
                    return Err(self.halt_at(None, Fault::bug("the run waits for nothing, and its function has not ended")));
                }
                self.wait(seen, until);
                continue;
            }
            let Some(wake) = wake else {
                return Err(self.halt_at(None, Fault::bug("the run waits for nothing, and its function has not ended")));
            };
            let next = [Some(wake), next_setting, self.settings.timeout].into_iter().flatten().min().unwrap_or(wake);
            self.now = self.now.max(next);
            self.at_this_time()?;
            if self.ending.is_some() {
                return Ok(());
            }
            self.wake_sleepers();
        }
    }

    /// In real time, the clock of the run follows the real one, never back, and never past its end.
    fn tick(&mut self) {
        if self.real_time.is_some() {
            let real = i64::try_from(self.control.real_ns()).unwrap_or(i64::MAX).min(CLOCK_END);
            self.now = self.now.max(real);
        }
    }

    /// In real time, waits until `until` on the clock of the run, or until something wakes the scheduler.
    fn wait(&mut self, seen: u64, until: Option<i64>) {
        let Some(wake) = self.real_time.as_ref().map(|real| real.wake.clone()) else { return };
        self.control.wait(&wake, seen, until.map(|until| u64::try_from(until).unwrap_or(0)));
    }

    /// The time `time.now` gives: the clock of the run, from 1970-01-01T00:00:00Z in deterministic mode (Q-21), from the
    /// start of the run in real time.
    pub(super) fn time(&self) -> i64 {
        let epoch = self.real_time.as_ref().map_or(0, |real| real.epoch);
        epoch.saturating_add(self.now)
    }

    /// The settings of `run.set_element`, applied now, before the nodes ready now: the reads that start after it see it. A
    /// value that does not fit in the memory of the run is refused and the run goes on (decision of the owner, 2026-11-11);
    /// every setting taken gets its answer, those after a fault that they never applied.
    fn take_live(&mut self) -> Result<(), Halt> {
        let mut taken = self.control.live().into_iter();
        while let Some(Live { tag, element, value }) = taken.next() {
            let setting = PanelSetting { element, value, at: self.now, by: SetBy::RunSetElement };
            match self.apply(setting.clone()) {
                Ok(()) => self.control.applied(tag, Some(self.now)),
                Err(fault) if fault.code == FaultCode::Memory => self.control.refused(tag, &fault),
                Err(fault) => {
                    // That one and those after it never apply, and `panel` of the report says so, as their answers do.
                    self.never.push(Applied { setting, applied: false });
                    self.control.applied(tag, None);
                    for Live { tag, element, value } in taken {
                        let setting = PanelSetting { element, value, at: self.now, by: SetBy::RunSetElement };
                        self.never.push(Applied { setting, applied: false });
                        self.control.applied(tag, None);
                    }
                    return Err(self.halt_at(None, fault));
                }
            }
        }
        Ok(())
    }

    /// Applies a setting of the top-level panel now, and keeps it for `panel` of the report, where its value counts in the
    /// memory of the run until the run ends, as a wire's does: once, however many hold it.
    fn apply(&mut self, mut setting: PanelSetting) -> Result<(), Fault> {
        if !self.memory.hold(&setting.value) {
            return Err(memory_fault("a panel element", crate::value::logical_size(&setting.value)));
        }
        if let Err(fault) = self.set_element(0, setting.element, setting.value.clone(), setting.by) {
            self.memory.release(&setting.value);
            return Err(fault);
        }
        setting.at = self.now;
        self.applied.push(Applied { setting, applied: true });
        Ok(())
    }

    /// The work of the pool that ended: each call is given what its work returned, and steps on in its place among the ready
    /// nodes. Work that panicked is the fault `LC8004` at its node, after every other has been taken.
    fn jobs_done(&mut self) -> Result<(), Halt> {
        if self.blocked == 0 {
            return Ok(());
        }
        let done = std::mem::take(&mut *self.jobs.lock().unwrap_or_else(PoisonError::into_inner));
        let mut fault = None;
        for JobDone { task, serial, out } in done {
            let Some(Task { instance, node, pos, state: State::Native(call) }) = self.task_mut(task) else { continue };
            if call.blocked != Some(serial) {
                continue;
            }
            call.blocked = None;
            let (instance, node, pos, library, id) = (*instance, *node, Rc::clone(pos), call.library, call.call);
            self.blocked = self.blocked.saturating_sub(1);
            let resumed = match (out, self.natives.get_mut(library)) {
                (Some(out), Some(slot)) => catch_unwind(AssertUnwindSafe(|| slot.run.resume(id, out))).is_ok(),
                _ => false,
            };
            if resumed {
                self.push_ready(pos, Ready::Resume(task));
                continue;
            }
            self.drop_call(task);
            if !self.stopping && fault.is_none() {
                let name = self.natives.get(library).map_or_else(String::new, |slot| slot.name.clone());
                fault = Some(self.halt(instance, node, panicked(&name)));
            }
        }
        fault.map_or(Ok(()), Err)
    }

    /// At one time on the clock: the settings of that time, then the timeout, before the nodes (cli.md 6.2).
    fn at_this_time(&mut self) -> Result<(), Halt> {
        while let Some(applied) = self.panel.get(self.next_setting) {
            if applied.setting.at > self.now {
                break;
            }
            // Dated when it applies, which in real time can be a little after the time it was given for.
            let setting = applied.setting.clone();
            self.apply(setting).map_err(|fault| self.halt_at(None, fault))?;
            self.next_setting += 1;
        }
        if self.settings.timeout.is_some_and(|timeout| timeout <= self.now) {
            self.abort(AbortBy::Timeout);
        }
        Ok(())
    }

    fn wake_sleepers(&mut self) {
        while let Some(Reverse((wake, _, task))) = self.sleepers.peek().copied() {
            if wake > self.now {
                break;
            }
            self.sleepers.pop();
            if let Some(pos) = self.task(task).map(|task| Rc::clone(&task.pos)) {
                self.push_ready(pos, Ready::Resume(task));
            }
        }
    }

    fn check_real_time(&mut self, ticket: &Ticket) -> Result<(), Halt> {
        let Some(limit) = self.settings.max_real_time else { return Ok(()) };
        if self.control.real_ns() <= limit {
            return Ok(());
        }
        let fault = Fault::new(
            FaultCode::RealTime,
            format!("the run has taken more than {} s of real time (--max-real-time), which depends on the speed of the machine", limit as f64 / 1e9),
        );
        Err(match ticket.what {
            Ready::Start { instance, node } => self.halt(instance, node, fault),
            Ready::Resume(task) => match self.task(task).map(|task| (task.instance, task.node)) {
                Some((instance, node)) => self.halt(instance, node, fault),
                None => self.halt_at(None, fault),
            },
        })
    }

    fn run_ticket(&mut self, ticket: Ticket) -> Result<(), Halt> {
        match ticket.what {
            Ready::Start { instance, node } => super::nodes::start(self, instance, node, ticket.pos),
            Ready::Resume(task) => match self.task(task).map(|task| matches!(task.state, State::Wait)) {
                Some(true) => {
                    let Some(task) = self.take_task(task) else { return Ok(()) };
                    self.finish(task.instance, task.node, vec![Value::NONE], false)
                }
                Some(false) => self.step_native(task),
                None => Ok(()),
            },
        }
    }

    /// The instances whose nodes have all ended give their results, iteratively, since an end can end what contains it.
    pub(super) fn drain(&mut self) -> Result<(), Halt> {
        while let Some(instance) = self.done.pop() {
            if self.ending.is_some() {
                return Ok(());
            }
            super::nodes::complete(self, instance)?;
        }
        Ok(())
    }

    pub(super) fn finished(&mut self, outputs: Vec<Value>) {
        self.ending = Some(Ending::Finished(outputs));
    }

    fn abort(&mut self, by: AbortBy) {
        if self.ending.is_some() {
            return;
        }
        let loops = self.loops_going();
        self.ending = Some(Ending::Aborted(AbortAt { by, at: self.now, loops }));
    }

    /// Each loop that runs, outermost first: by the position of its node.
    fn loops_going(&self) -> Vec<LoopAt> {
        let mut going: Vec<(Pos, LoopAt)> = self
            .tasks
            .iter()
            .flatten()
            .filter_map(|task| {
                let iteration = match &task.state {
                    State::Loop(state) => state.iteration,
                    State::For(state) if state.parallel => state.ended,
                    State::For(state) => state.iteration.saturating_sub(1),
                    _ => return None,
                };
                let instance = self.instance(task.instance)?;
                let node = instance.region.nodes.get(task.node as usize)?;
                let function = self.frame(instance.frame)?.function;
                Some((Rc::clone(&task.pos), LoopAt { function, node: node.name, iteration }))
            })
            .collect();
        going.sort_by(|a, b| compare(&a.0, &b.0));
        going.into_iter().map(|(_, at)| at).collect()
    }

    // Winding down (spec 8.2) ------------------------------------------------------------------------------------------------

    /// After an abort or a fault: no node starts, waits end at once, native calls in progress go on until the grace period
    /// ends, on the clock of the run; those still going then are abandoned, and their resources reported as leaked. Then each
    /// resource the run opened is released by its library.
    fn wind_down(&mut self) {
        self.stopping = true;
        let deadline = self.now.saturating_add(self.settings.grace.max(0)).min(CLOCK_END);
        let natives: Vec<Ticket> = self.ready.drain().into_iter().filter(|ticket| self.is_native(ticket.what)).collect();
        for ticket in natives {
            self.ready.push(ticket);
        }
        let sleeping: Vec<Reverse<(i64, u64, TaskId)>> =
            std::mem::take(&mut self.sleepers).into_iter().filter(|Reverse((_, _, task))| self.is_native(Ready::Resume(*task))).collect();
        self.sleepers = sleeping.into_iter().collect();
        // Ended from outside, the grace period ends now, not at its deadline.
        let mut cut = false;
        loop {
            let seen = self.real_time.as_ref().map(|real| real.wake.seen());
            self.tick();
            // A library that never ends a call, asking to wait 0 ns again and again, is stopped by the real time of the run.
            if self.settings.max_real_time.is_some_and(|limit| self.control.real_ns() > limit) {
                break;
            }
            if self.control.grace_over() {
                cut = true;
                break;
            }
            if seen.is_some() {
                let _ = self.jobs_done();
                if self.now >= deadline {
                    break;
                }
                self.wake_sleepers();
            }
            if let Some(ticket) = self.ready.pop() {
                if let Ready::Resume(task) = ticket.what {
                    let _ = self.step_native(task);
                }
                continue;
            }
            let wake = self.sleepers.peek().map(|Reverse((wake, _, _))| *wake).filter(|wake| *wake <= deadline);
            if let Some(seen) = seen {
                // A call that sleeps past the deadline, or whose work is on the pool, is waited for until the deadline.
                if self.sleepers.is_empty() && self.blocked == 0 {
                    break;
                }
                self.wait(seen, Some(wake.unwrap_or(deadline)));
                continue;
            }
            match wake {
                Some(wake) => {
                    self.now = self.now.max(wake);
                    self.wake_sleepers();
                }
                None => break,
            }
        }
        let abandoned: Vec<TaskId> = (0..self.tasks.len()).filter(|task| self.is_native(Ready::Resume(*task))).collect();
        if !abandoned.is_empty() && !cut {
            self.now = self.now.max(deadline);
        }
        for task in abandoned {
            let Some(Task { state: State::Native(call), .. }) = self.take_task(task) else { continue };
            if let Some(slot) = self.natives.get_mut(call.library) {
                let _ = catch_unwind(AssertUnwindSafe(|| slot.run.cancel(call.call)));
            }
            for value in &call.inputs {
                self.memory.release(value);
            }
            for handle in handles_in(&call.inputs) {
                if let Some(index) = self.opened_index.get(&handle).copied()
                    && let Some(opened) = self.opened.get_mut(index)
                {
                    opened.leaked = true;
                }
            }
        }
        for index in 0..self.opened.len() {
            let Some(opened) = self.opened.get(index) else { continue };
            if opened.leaked {
                continue;
            }
            let handle = opened.handle;
            let Some(slot) = self.natives.get_mut(handle.library as usize) else { continue };
            let open = catch_unwind(AssertUnwindSafe(|| slot.run.describe(handle))).ok().flatten().is_some_and(|info| info.open);
            if open {
                let _ = catch_unwind(AssertUnwindSafe(|| slot.run.release(handle)));
                if let Some(opened) = self.opened.get_mut(index) {
                    opened.released = true;
                }
            }
        }
    }

    fn is_native(&self, what: Ready) -> bool {
        match what {
            Ready::Resume(task) => matches!(self.task(task).map(|task| &task.state), Some(State::Native(_))),
            Ready::Start { .. } => false,
        }
    }

    fn end(mut self) -> RunEnd {
        let (status, outputs, fault, abort) = match self.ending.take() {
            Some(Ending::Finished(outputs)) => {
                let failed = self.program.functions.first().is_some_and(|function| {
                    function.outputs.iter().zip(&outputs).any(|(port, value)| Some(port.name) == self.err_name && value.as_error().is_some())
                });
                (if failed { Status::Error } else { Status::Ok }, outputs, None, None)
            }
            Some(Ending::Aborted(abort)) => (Status::Aborted, Vec::new(), None, Some(abort)),
            Some(Ending::Faulted(fault)) => (Status::Fault, Vec::new(), Some(fault), None),
            None => {
                let fault = *self.halt_at(None, Fault::bug("a run that ended without a status"));
                (Status::Fault, Vec::new(), Some(fault), None)
            }
        };
        let resources = self
            .opened
            .iter()
            .map(|opened| {
                let info = self
                    .natives
                    .get(opened.handle.library as usize)
                    .and_then(|slot| catch_unwind(AssertUnwindSafe(|| slot.run.describe(opened.handle))).ok().flatten());
                Resource {
                    handle: opened.handle,
                    info,
                    function: opened.function,
                    node: opened.node,
                    origin: opened.origin,
                    released: opened.released,
                    leaked: opened.leaked,
                }
            })
            .collect();
        let mut instruments = Vec::new();
        let mut logs = Vec::new();
        for slot in &self.natives {
            if let Ok(mut logged) = catch_unwind(AssertUnwindSafe(|| slot.run.instruments())) {
                instruments.append(&mut logged);
            }
            if let Ok(kept) = catch_unwind(AssertUnwindSafe(|| slot.run.logs()))
                && (!kept.entries.is_empty() || kept.more > 0)
            {
                logs.push((slot.name.clone(), kept));
            }
        }
        instruments.sort_by(|a, b| a.resource.cmp(&b.resource));
        // The settings in the order they applied, then those that never did: `set` after the end, and `run.set_element`
        // given as the run ended.
        let mut panel = std::mem::take(&mut self.applied);
        for applied in &panel {
            self.memory.release(&applied.setting.value);
        }
        let pending = std::mem::take(&mut self.panel);
        panel.extend(pending.into_iter().skip(self.next_setting));
        panel.append(&mut self.never);
        for Live { tag, element, value } in self.control.close_live() {
            self.control.applied(tag, None);
            panel.push(Applied { setting: PanelSetting { element, value, at: self.now, by: SetBy::RunSetElement }, applied: false });
        }
        RunEnd {
            status,
            outputs,
            fault,
            abort,
            clock: self.now,
            resources,
            instruments,
            logs,
            panel,
            nodes: self.nodes_run,
            held: self.memory.held(),
        }
    }

    // Faults -------------------------------------------------------------------------------------------------------------

    /// A fault at a node of an instance: its function, its line, the loops around it.
    pub(super) fn halt(&self, instance: InstanceId, node: u32, fault: Fault) -> Halt {
        let Some(found) = self.instance(instance) else { return self.halt_at(None, fault) };
        let function = self.frame(found.frame).map_or(FunctionId(0), |frame| frame.function);
        let loops = loops_of(found.loops.as_ref());
        match found.region.nodes.get(node as usize) {
            Some(node) => Box::new(FaultAt { fault, function, node: Some(node.name), origin: Some(node.origin), loops, iteration: None }),
            None => Box::new(FaultAt { fault, function, node: None, origin: None, loops, iteration: None }),
        }
    }

    /// A fault at no node: the top-level function.
    pub(super) fn halt_at(&self, function: Option<FunctionId>, fault: Fault) -> Halt {
        Box::new(FaultAt { fault, function: function.unwrap_or(FunctionId(0)), node: None, origin: None, loops: Vec::new(), iteration: None })
    }

    // Instances ----------------------------------------------------------------------------------------------------------

    fn plan(&mut self, region: &'p Region) -> Result<Rc<Plan>, Fault> {
        let key: *const Region = region;
        if let Some(plan) = self.plans.get(&key) {
            return Ok(Rc::clone(plan));
        }
        let plan = Rc::new(Plan::new(region)?);
        self.plans.insert(key, Rc::clone(&plan));
        Ok(plan)
    }

    /// Starts a region on its arguments: it counts 256 bytes for itself and for each of its nodes (cli.md 6.2), its nodes
    /// that wait for none are ready, and a region of no node ends at once.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new_instance(
        &mut self,
        region: &'p Region,
        args: Vec<Value>,
        frame: FrameId,
        pos: Pos,
        iter: Rc<[u64]>,
        loops: Option<Rc<LoopLink>>,
        parent: Parent,
    ) -> Result<InstanceId, Fault> {
        self.new_instance_with(region, args, 0, frame, pos, iter, loops, parent)
    }

    /// [`Self::new_instance`], whose first `free` arguments are not counted.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new_instance_with(
        &mut self,
        region: &'p Region,
        args: Vec<Value>,
        free: usize,
        frame: FrameId,
        pos: Pos,
        iter: Rc<[u64]>,
        loops: Option<Rc<LoopLink>>,
        parent: Parent,
    ) -> Result<InstanceId, Fault> {
        if args.len() != region.args.len() {
            return Err(Fault::bug("a region started with the wrong number of arguments"));
        }
        let plan = self.plan(region)?;
        let cost = NODE_COST.saturating_mul(u64::try_from(region.nodes.len()).unwrap_or(u64::MAX).saturating_add(1));
        if !self.memory.charge(cost) {
            return Err(memory_fault("a region that starts", cost));
        }
        let mut slots: Vec<Option<Value>> = vec![None; plan.slots];
        for (index, value) in args.into_iter().enumerate() {
            let reads = plan.reads.get(index).copied().unwrap_or(0);
            if reads == 0 {
                continue;
            }
            if index >= free && !self.memory.hold(&value) {
                for held in slots.iter().skip(free).flatten() {
                    self.memory.release(held);
                }
                self.memory.refund(cost);
                return Err(memory_fault("the values that enter a region", crate::value::logical_size(&value)));
            }
            if let Some(slot) = slots.get_mut(index) {
                *slot = Some(value);
            }
        }
        let instance = Instance {
            region,
            left: plan.reads.clone(),
            missing: plan.missing.clone(),
            pending: region.nodes.len(),
            plan: Rc::clone(&plan),
            slots,
            frame,
            pos,
            iter,
            loops,
            parent,
            cost,
            free,
        };
        let id = match self.free_instances.pop() {
            Some(id) => {
                if let Some(slot) = self.instances.get_mut(id) {
                    *slot = Some(instance);
                }
                id
            }
            None => {
                self.instances.push(Some(instance));
                self.instances.len() - 1
            }
        };
        if region.nodes.is_empty() {
            self.done.push(id);
        }
        for node in &plan.starts {
            self.ready_node(id, *node);
        }
        Ok(id)
    }

    pub(super) fn instance(&self, id: InstanceId) -> Option<&Instance<'p>> {
        self.instances.get(id).and_then(Option::as_ref)
    }

    fn instance_mut(&mut self, id: InstanceId) -> Option<&mut Instance<'p>> {
        self.instances.get_mut(id).and_then(Option::as_mut)
    }

    fn ready_node(&mut self, instance: InstanceId, node: u32) {
        let Some(found) = self.instance(instance) else { return };
        let Some(position) = found.region.nodes.get(node as usize).map(|node| node.position) else { return };
        let pos = child(&found.pos, u64::from(position));
        self.push_ready(pos, Ready::Start { instance, node });
    }

    fn push_ready(&mut self, pos: Pos, what: Ready) {
        self.seq = self.seq.wrapping_add(1);
        self.ready.push(Ticket { pos, seq: self.seq, what });
    }

    /// Reads one input of a node: the last read of a wire takes its value, so that a body can change it in place (D-11).
    pub(super) fn take(&mut self, instance: InstanceId, used: &labcraft_ir::Use) -> Result<Value, Fault> {
        use labcraft_ir::Source;
        let raw = match &used.source {
            Source::Const(id) => constant(self.env(), *id)?,
            Source::Array(items) => {
                let mut values = Vec::with_capacity(items.len());
                for item in items {
                    values.push(self.take(instance, item)?);
                }
                return super::nodes::array_of(self.env(), values, used.ty);
            }
            source => {
                let Some(found) = self.instances.get_mut(instance).and_then(Option::as_mut) else {
                    return Err(Fault::bug("a read in a region that has ended"));
                };
                let slot = found.plan.slot(source).ok_or_else(|| Fault::bug("a read of a value that its region has not"))?;
                let left = found.left.get_mut(slot).ok_or_else(|| Fault::bug("a read past the slots of a region"))?;
                *left = left.saturating_sub(1);
                let last = *left == 0;
                let counted = slot >= found.free;
                let value = found.slots.get_mut(slot).ok_or_else(|| Fault::bug("a read past the slots of a region"))?;
                if last {
                    let value = value.take().ok_or_else(|| Fault::bug("a value read before it was made, or once too often"))?;
                    if counted {
                        self.memory.release_taken(&value);
                    }
                    value
                } else {
                    value.clone().ok_or_else(|| Fault::bug("a value read before it was made"))?
                }
            }
        };
        read(&raw, used)
    }

    /// A node ended: its outputs are counted and kept for their reads, and the nodes that waited for it count down.
    pub(super) fn finish(&mut self, instance: InstanceId, node: u32, outputs: Vec<Value>, skipped: bool) -> Result<(), Halt> {
        if self.stopping {
            return Ok(());
        }
        let Some(found) = self.instance(instance) else { return Err(self.halt_at(None, Fault::bug("a node of a region that has ended"))) };
        let region = found.region;
        let plan = Rc::clone(&found.plan);
        let frame_number = self.frame(found.frame).map_or(0, |frame| frame.number);
        let iter = Rc::clone(&found.iter);
        let Some(the_node) = region.nodes.get(node as usize) else { return Err(self.halt(instance, node, Fault::bug("no such node"))) };
        if outputs.len() != the_node.outputs.len() {
            let message =
                format!("node `{}` gave {} values for {} outputs", self.names.resolve(the_node.name), outputs.len(), the_node.outputs.len());
            return Err(self.halt(instance, node, Fault::bug(message)));
        }
        for (index, value) in outputs.iter().enumerate() {
            if !self.memory.hold_output(value) {
                for held in outputs.iter().take(index) {
                    self.memory.release(held);
                }
                let size = crate::value::logical_size(value);
                return Err(self.halt(instance, node, memory_fault("the outputs of this node", size)));
            }
        }
        if let Some((made, limit)) = self.memory.made_over() {
            for value in &outputs {
                self.memory.release(value);
            }
            let message = format!("the nodes of the run have made {made} bytes of values, past its limit of {limit} (--max-made)");
            return Err(self.halt(instance, node, Fault::new(FaultCode::Made, message)));
        }
        self.observer.ran(self.now, frame_number, the_node, &iter, Outputs { values: &outputs, skipped }, &Described(&self.natives));
        let start = plan.outputs(node).unwrap_or(0);
        for (index, value) in outputs.into_iter().enumerate() {
            let slot = start + index;
            if plan.reads.get(slot).copied().unwrap_or(0) == 0 {
                self.memory.release(&value);
                continue;
            }
            if let Some(place) = self.instance_mut(instance).and_then(|found| found.slots.get_mut(slot)) {
                *place = Some(value);
            }
        }
        let mut ready = Vec::new();
        let mut ended = false;
        let dependents = plan.dependents.get(node as usize).map_or(&[][..], Vec::as_slice);
        if let Some(found) = self.instances.get_mut(instance).and_then(Option::as_mut) {
            for &dependent in dependents {
                if let Some(missing) = found.missing.get_mut(dependent as usize) {
                    *missing = missing.saturating_sub(1);
                    if *missing == 0 {
                        ready.push(dependent);
                    }
                }
            }
            found.pending = found.pending.saturating_sub(1);
            ended = found.pending == 0;
        }
        for dependent in ready {
            self.ready_node(instance, dependent);
        }
        if ended {
            self.done.push(instance);
        }
        Ok(())
    }

    /// The results of an instance whose nodes have all ended; the instance is gone after.
    pub(super) fn results(&mut self, instance: InstanceId) -> Result<(Vec<Value>, Parent), Fault> {
        let region = self.instance(instance).map(|found| found.region).ok_or_else(|| Fault::bug("the results of a region that has ended"))?;
        let mut results = Vec::with_capacity(region.results.len());
        for used in &region.results {
            results.push(self.take(instance, used)?);
        }
        let found = self.instances.get_mut(instance).and_then(Option::take).ok_or_else(|| Fault::bug("a region that ends twice"))?;
        for value in found.slots.iter().skip(found.free).flatten() {
            self.memory.release(value);
        }
        self.memory.refund(found.cost);
        self.free_instances.push(instance);
        Ok((results, found.parent))
    }

    // Tasks --------------------------------------------------------------------------------------------------------------

    pub(super) fn new_task(&mut self, task: Task<'p>) -> TaskId {
        match self.free_tasks.pop() {
            Some(id) => {
                if let Some(slot) = self.tasks.get_mut(id) {
                    *slot = Some(task);
                }
                id
            }
            None => {
                self.tasks.push(Some(task));
                self.tasks.len() - 1
            }
        }
    }

    pub(super) fn task(&self, id: TaskId) -> Option<&Task<'p>> {
        self.tasks.get(id).and_then(Option::as_ref)
    }

    pub(super) fn task_mut(&mut self, id: TaskId) -> Option<&mut Task<'p>> {
        self.tasks.get_mut(id).and_then(Option::as_mut)
    }

    pub(super) fn take_task(&mut self, id: TaskId) -> Option<Task<'p>> {
        let task = self.tasks.get_mut(id).and_then(Option::take)?;
        self.free_tasks.push(id);
        Some(task)
    }

    /// A node sleeps for `ns` on the clock of the run: past its end, `LC8007`.
    pub(super) fn sleep(&mut self, task: TaskId, ns: i64) -> Result<(), Fault> {
        let wake = self.now.checked_add(ns.max(0)).filter(|wake| *wake <= CLOCK_END).ok_or_else(|| {
            Fault::new(FaultCode::Clock, format!("a wait of {} s would take the clock of the run past its end, 2^62 ns", ns as f64 / 1e9))
        })?;
        if wake == self.now {
            // Ready again at this time: it takes its place among the ready nodes (spec 8), rather than after all of them.
            if let Some(pos) = self.task(task).map(|task| Rc::clone(&task.pos)) {
                self.push_ready(pos, Ready::Resume(task));
            }
            return Ok(());
        }
        self.seq = self.seq.wrapping_add(1);
        self.sleepers.push(Reverse((wake, self.seq, task)));
        Ok(())
    }

    // Native calls -------------------------------------------------------------------------------------------------------

    /// Starts a call of an `extern rust` function in its library, then steps it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_native(
        &mut self,
        instance: InstanceId,
        node: u32,
        pos: Pos,
        library: usize,
        function: &str,
        inputs: Vec<Value>,
        failed: (Vec<Value>, &'p Skip),
    ) -> Result<(), Halt> {
        let Some(slot) = self.natives.get_mut(library) else { return Err(self.halt(instance, node, Fault::bug("a native call of no library"))) };
        let started = catch_unwind(AssertUnwindSafe(|| slot.run.call(function, inputs)));
        let name = slot.name.clone();
        let call = match started {
            Ok(Some(call)) => call,
            Ok(None) => {
                let message = format!("the library `{name}` has no native function `{function}`");
                return Err(self.halt(instance, node, Fault::bug(message)));
            }
            Err(_) => return Err(self.halt(instance, node, panicked(&name))),
        };
        let (inputs, skip) = failed;
        // What the library was given stays held while the call goes on: what it does not keep, it lets go when the call ends.
        for (at, value) in inputs.iter().enumerate() {
            if !self.memory.hold(value) {
                for held in inputs.iter().take(at) {
                    self.memory.release(held);
                }
                if let Some(slot) = self.natives.get_mut(library) {
                    let _ = catch_unwind(AssertUnwindSafe(|| slot.run.cancel(call)));
                }
                return Err(self.halt(instance, node, memory_fault("the inputs of a native call", crate::value::logical_size(value))));
            }
        }
        let task = self.new_task(Task {
            instance,
            node,
            pos,
            state: State::Native(NativeCall { library, call, blocked: None, held: Vec::new(), inputs, skip }),
        });
        self.step_native(task)
    }

    /// Asks a call for its next steps until it waits or ends.
    fn step_native(&mut self, task: TaskId) -> Result<(), Halt> {
        loop {
            let Some(Task { instance, node, state: State::Native(call), .. }) = self.task(task) else { return Ok(()) };
            let (instance, node, library, id) = (*instance, *node, call.library, call.call);
            let now = self.now;
            let Some(slot) = self.natives.get_mut(library) else { return Err(self.halt(instance, node, Fault::bug("a native call of no library"))) };
            let step = match catch_unwind(AssertUnwindSafe(|| slot.run.step(id, CallContext { now }))) {
                Ok(step) => step,
                Err(_) => {
                    let fault = panicked(&slot.name);
                    self.drop_call(task);
                    if self.stopping {
                        return Ok(());
                    }
                    return Err(self.halt(instance, node, fault));
                }
            };
            match step {
                Step::Sleep(ns) => {
                    return self.sleep(task, ns).map_err(|fault| self.halt(instance, node, fault));
                }
                Step::Exclusive(key) => match self.holders.get(&key) {
                    Some(holder) if *holder != task => {
                        let pos = self.task(task).map(|task| Rc::clone(&task.pos)).unwrap_or_else(root);
                        self.seq = self.seq.wrapping_add(1);
                        let seq = self.seq;
                        self.waiting.entry(key).or_default().push(Waiter { pos, seq, task });
                        return Ok(());
                    }
                    Some(_) => {}
                    None => {
                        self.holders.insert(key, task);
                        if let Some(Task { state: State::Native(call), .. }) = self.task_mut(task) {
                            call.held.push(key);
                        }
                    }
                },
                Step::Release(key) => {
                    if let Some(Task { state: State::Native(call), .. }) = self.task_mut(task) {
                        call.held.retain(|held| *held != key);
                    }
                    self.give(key);
                }
                Step::Blocking(job) => {
                    if !self.block(task, job)? {
                        return Ok(());
                    }
                }
                Step::Done(outcome) => {
                    let Some(Task { state: State::Native(call), .. }) = self.take_task(task) else { return Ok(()) };
                    for key in &call.held {
                        self.give(*key);
                    }
                    for value in &call.inputs {
                        self.memory.release(value);
                    }
                    return self.native_done(instance, node, call, outcome);
                }
            }
        }
    }

    /// The work of a call that blocks: on the pool in real time, its call waiting for it; at once on this thread otherwise,
    /// the call then given what it returned. Whether the call steps on at once.
    fn block(&mut self, task: TaskId, job: Job) -> Result<bool, Halt> {
        let Some(Task { instance, node, state: State::Native(call), .. }) = self.task(task) else { return Ok(false) };
        let (instance, node, library, id) = (*instance, *node, call.library, call.call);
        if let Some((pool, wake)) = self.real_time.as_ref().map(|real| (real.pool.clone(), real.wake.clone())) {
            self.serial = self.serial.wrapping_add(1);
            let serial = self.serial;
            if let Some(Task { state: State::Native(call), .. }) = self.task_mut(task) {
                call.blocked = Some(serial);
            }
            self.blocked += 1;
            let jobs = Arc::clone(&self.jobs);
            let work: Work = Box::new(move || {
                let out = catch_unwind(AssertUnwindSafe(job)).ok();
                jobs.lock().unwrap_or_else(PoisonError::into_inner).push(JobDone { task, serial, out });
                wake.wake();
            });
            // No thread could start: the work runs here, and the scheduler takes its end as any other.
            if let Err(work) = pool.submit(work) {
                work();
            }
            return Ok(false);
        }
        let out = catch_unwind(AssertUnwindSafe(job));
        let Some(slot) = self.natives.get_mut(library) else { return Err(self.halt(instance, node, Fault::bug("a native call of no library"))) };
        let resumed = match out {
            Ok(out) => catch_unwind(AssertUnwindSafe(|| slot.run.resume(id, out))).is_ok(),
            Err(_) => false,
        };
        if resumed {
            return Ok(true);
        }
        let fault = panicked(&slot.name);
        self.drop_call(task);
        if self.stopping {
            return Ok(false);
        }
        Err(self.halt(instance, node, fault))
    }

    /// A call whose library panicked ends there: it gives back its keys and what it held, and the resources it was given are
    /// released with the others, as after any fault (10.4), rather than left as leaked.
    fn drop_call(&mut self, task: TaskId) {
        let Some(Task { state: State::Native(call), .. }) = self.take_task(task) else { return };
        if call.blocked.is_some() {
            self.blocked = self.blocked.saturating_sub(1);
        }
        for key in &call.held {
            self.give(*key);
        }
        for value in &call.inputs {
            self.memory.release(value);
        }
    }

    /// A key given back goes to the call that waits for it first in the global order.
    fn give(&mut self, key: Key) {
        self.holders.remove(&key);
        let Some(waiters) = self.waiting.get_mut(&key) else { return };
        // A waiter whose call has ended since is passed over: each is popped once, so a hand-over costs a step of a heap.
        let mut first = None;
        while let Some(waiter) = waiters.pop() {
            if matches!(self.tasks.get(waiter.task), Some(Some(Task { state: State::Native(_), .. }))) {
                first = Some(waiter);
                break;
            }
        }
        if waiters.is_empty() {
            self.waiting.remove(&key);
        }
        let Some(Waiter { pos, task: waiter, .. }) = first else { return };
        self.holders.insert(key, waiter);
        if let Some(Task { state: State::Native(call), .. }) = self.task_mut(waiter) {
            call.held.push(key);
        }
        self.push_ready(pos, Ready::Resume(waiter));
    }

    fn native_done(&mut self, instance: InstanceId, node: u32, call: NativeCall<'p>, outcome: NativeOutcome) -> Result<(), Halt> {
        let outputs = match outcome {
            NativeOutcome::Outputs(mut outputs) => {
                self.record_handles(instance, node, &outputs);
                // An `always` node that ran passes on the error it received on its `err` output (spec 7.2): the library does
                // not see it.
                if let Skip::OnError { err, always: true, through, .. } = call.skip
                    && let Some(received) = call.inputs.get(*err as usize).filter(|value| value.as_error().is_some())
                {
                    for (output, passes) in outputs.iter_mut().zip(through) {
                        if *passes == Some(*err) {
                            *output = received.clone();
                        }
                    }
                }
                outputs
            }
            NativeOutcome::Fail { category, code, message } => {
                if self.stopping {
                    return Ok(());
                }
                let error = self.error_at(instance, node, category, code, message);
                let types = self.output_types(instance, node);
                let env = self.env();
                let host = super::nodes::Quiet { room: self.memory.room() };
                labcraft_ir_outputs(env, call.skip, &call.inputs, &types, error, &host).map_err(|fault| self.halt(instance, node, fault))?
            }
        };
        self.finish(instance, node, outputs, false)
    }

    pub(super) fn output_types(&self, instance: InstanceId, node: u32) -> Vec<labcraft_types::TyId> {
        self.instance(instance)
            .and_then(|found| found.region.nodes.get(node as usize))
            .map(|node| node.outputs.iter().map(|output| output.ty).collect())
            .unwrap_or_default()
    }

    fn record_handles(&mut self, instance: InstanceId, node: u32, outputs: &[Value]) {
        let found = self.instance(instance);
        let function = found.and_then(|found| self.frame(found.frame)).map_or(FunctionId(0), |frame| frame.function);
        let (name, origin) =
            found.and_then(|found| found.region.nodes.get(node as usize)).map_or((None, None), |node| (Some(node.name), Some(node.origin)));
        for handle in handles_in(outputs) {
            if self.opened_index.contains_key(&handle) || self.natives.get(handle.library as usize).is_none() {
                continue;
            }
            self.opened_index.insert(handle, self.opened.len());
            self.opened.push(Opened { handle, function, node: name, origin, released: false, leaked: false });
        }
    }

    // Frames, panels and errors ------------------------------------------------------------------------------------------

    /// A frame for a call: its panel starts from its inputs and the defaults of its elements (spec 10).
    pub(super) fn new_frame(
        &mut self,
        function: FunctionId,
        parent: Option<FrameId>,
        site: Option<Place>,
        depth: u32,
        inputs: &[Value],
    ) -> Result<FrameId, Fault> {
        let declared = self.program.functions.get(function.0 as usize).ok_or_else(|| Fault::bug("a call of no function"))?;
        if !self.memory.charge(NODE_COST) {
            return Err(memory_fault("a call", NODE_COST));
        }
        let mut panel = Vec::with_capacity(declared.elements.len());
        for element in &declared.elements {
            let value = match (element.port, element.default) {
                (Some(labcraft_ir::PortOf::Input(index)), _) => inputs.get(index as usize).cloned(),
                (_, Some(default)) => Some(constant(self.env(), default)?),
                _ => None,
            };
            if let Some(value) = &value
                && !self.memory.hold(value)
            {
                return Err(memory_fault("the panel of a call", crate::value::logical_size(value)));
            }
            panel.push(value);
        }
        let frame = Frame { function, number: self.next_frame, depth, panel, parent, site, callers: None };
        self.next_frame += 1;
        Ok(match self.free_frames.pop() {
            Some(id) => {
                if let Some(slot) = self.frames.get_mut(id) {
                    *slot = Some(frame);
                }
                id
            }
            None => {
                self.frames.push(Some(frame));
                self.frames.len() - 1
            }
        })
    }

    pub(super) fn frame(&self, id: FrameId) -> Option<&Frame> {
        self.frames.get(id).and_then(Option::as_ref)
    }

    pub(super) fn end_frame(&mut self, id: FrameId) {
        let Some(frame) = self.frames.get_mut(id).and_then(Option::take) else { return };
        for value in frame.panel.iter().flatten() {
            self.memory.release(value);
        }
        self.memory.refund(NODE_COST);
        self.free_frames.push(id);
    }

    /// Reads an element of the panel of a frame: a `button` that is pressed is seen by one read (spec 10).
    pub(super) fn element(&mut self, frame: FrameId, element: labcraft_ir::ElementId) -> Result<Value, Fault> {
        let function = self.frame(frame).map(|frame| frame.function).ok_or_else(|| Fault::bug("a panel of no call"))?;
        let kind = self
            .program
            .functions
            .get(function.0 as usize)
            .and_then(|function| function.elements.get(element.0 as usize))
            .map(|element| element.kind);
        let button = kind.is_some() && kind == self.button;
        let Some(Some(frame)) = self.frames.get_mut(frame) else { return Err(Fault::bug("a panel of no call")) };
        let slot = frame.panel.get_mut(element.0 as usize).ok_or_else(|| Fault::bug("an element that its panel has not"))?;
        let value = slot.clone().ok_or_else(|| Fault::bug("an element whose type is not settled, read"))?;
        if button && matches!(value, Value::Bool(true)) {
            *slot = Some(Value::Bool(false));
        }
        Ok(value)
    }

    pub(super) fn set_element(&mut self, frame: FrameId, element: labcraft_ir::ElementId, value: Value, by: SetBy) -> Result<(), Fault> {
        if !self.memory.hold(&value) {
            return Err(memory_fault("a panel element", crate::value::logical_size(&value)));
        }
        let now = self.now;
        let Some(Some(found)) = self.frames.get_mut(frame) else {
            self.memory.release(&value);
            return Err(Fault::bug("a panel of no call"));
        };
        let number = found.number;
        let Some(slot) = found.panel.get_mut(element.0 as usize) else {
            self.memory.release(&value);
            return Err(Fault::bug("an element that its panel has not"));
        };
        let old = slot.replace(value.clone());
        if let Some(old) = old {
            self.memory.release(&old);
        }
        self.observer.set(now, number, element, &value, by, &Described(&self.natives));
        Ok(())
    }

    /// Where a node is, as an error names it (commands.md 5).
    pub(super) fn place(&self, function: FunctionId, node: &Node) -> Place {
        let (shown, file) = self.shown.get(function.0 as usize).cloned().unwrap_or_else(|| (Arc::from(""), Arc::from("")));
        let line = self
            .program
            .functions
            .get(function.0 as usize)
            .and_then(|function| self.files.get(function.file as usize))
            .and_then(|file| file.source_map.span(node.origin))
            .map_or(0, |span| u64::from(span.start.line));
        Place { function: shown, node: Arc::from(self.names.resolve(node.name)), file, line }
    }

    pub(super) fn frame_parent(&self, id: FrameId) -> Option<FrameId> {
        self.frame(id).and_then(|frame| frame.parent)
    }

    /// The callers of a frame, innermost first, as they are now, without keeping them.
    pub(super) fn callers_now(&self, frame: FrameId) -> Arc<[Place]> {
        if let Some(callers) = self.frame(frame).and_then(|frame| frame.callers.clone()) {
            return callers;
        }
        let mut places = Vec::new();
        let mut at = Some(frame);
        while let Some(id) = at {
            let Some(found) = self.frame(id) else { break };
            if let Some(site) = &found.site {
                places.push(site.clone());
            }
            at = found.parent;
        }
        places.into()
    }

    /// The callers of a frame, innermost first, made once for the frame.
    fn callers(&mut self, frame: FrameId) -> Arc<[Place]> {
        if let Some(callers) = self.frame(frame).and_then(|frame| frame.callers.clone()) {
            return callers;
        }
        let mut places = Vec::new();
        let mut at = Some(frame);
        while let Some(id) = at {
            let Some(found) = self.frame(id) else { break };
            if let Some(site) = &found.site {
                places.push(site.clone());
            }
            at = found.parent;
        }
        let callers: Arc<[Place]> = places.into();
        if let Some(Some(found)) = self.frames.get_mut(frame) {
            found.callers = Some(Arc::clone(&callers));
        }
        callers
    }

    /// A new error, made at a node: its origin is the node, then the calls that led to it.
    pub(super) fn error_at(&mut self, instance: InstanceId, node: u32, category: &str, code: i64, message: String) -> Value {
        let found = self.instance(instance).map(|found| (found.frame, found.region));
        let origin = found.and_then(|(frame, region)| {
            let function = self.frame(frame)?.function;
            let node = region.nodes.get(node as usize)?;
            Some((frame, self.place(function, node)))
        });
        let origin = origin.map(|(frame, place)| Arc::new(Origin { place, callers: self.callers(frame) }));
        Value::error(ErrorData { category: Arc::from(category), code, message: Arc::from(message), origin, others: Arc::default() })
    }

    pub(super) fn is_error_type(&self, ty: labcraft_types::TyId) -> bool {
        matches!(self.env().ty(ty), Ty::ErrorValue)
    }
}

fn loops_of(link: Option<&Rc<LoopLink>>) -> Vec<LoopAt> {
    let mut loops = Vec::new();
    let mut at = link;
    while let Some(found) = at {
        loops.push(LoopAt { function: found.function, node: found.node, iteration: found.iteration });
        at = found.parent.as_ref();
    }
    loops.reverse();
    loops
}

pub(super) fn memory_fault(what: &str, size: u64) -> Fault {
    Fault::memory(format!("{what} would hold {size} more bytes, beyond the memory of the run (--max-memory, cli.md 6.2)"))
}

pub(super) fn panicked(library: &str) -> Fault {
    Fault::bug(format!("the library `{library}` panicked; this is a bug of the library"))
}

/// The handles a value holds, at any depth, with an explicit stack.
fn handles_in(values: &[Value]) -> Vec<Handle> {
    let mut found = Vec::new();
    let mut pending: Vec<&Value> = values.iter().collect();
    while let Some(value) = pending.pop() {
        match value {
            Value::Handle(handle) if handle.is_valid() => found.push(*handle),
            Value::Array(items) => pending.extend(items.items()),
            Value::Grid(grid) => pending.extend(grid.items()),
            Value::Struct(fields) => pending.extend(fields.iter()),
            Value::Variant(variant) => pending.extend(variant.values.iter()),
            _ => {}
        }
    }
    found
}

fn labcraft_ir_outputs(
    env: Env<'_>,
    skip: &Skip,
    inputs: &[Value],
    types: &[labcraft_types::TyId],
    error: Value,
    host: &dyn crate::primitives::Host,
) -> Result<Vec<Value>, Fault> {
    crate::primitives::outputs_on_error(env, skip, inputs, types, Some(error), host)
}
