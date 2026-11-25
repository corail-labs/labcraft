# The window of the IDE (M1)

*Step 6 of the specifications of M1, decided with the owner on 2026-11-15. What the window shows, where, and what the
agent attached to it sees and does. The canvas and its gestures are in [visual-notation.md](visual-notation.md)
(sections 2.4 to 2.6), the editing commands in [commands.md](commands.md) (section 9), the server in
[mcp.md](mcp.md). A value marked (proposed) is settled when the Rust of M1 is written, after it has been tried in
the window.*

The window follows the public conventions of desktop IDEs and of the Craft apps; it borrows no look, icon or text from
NI (D-23). Its pieces that another IDE can use are written for that from the start (section 11).

## 1. Principles

- **Everything is a command.** A menu entry, a key, a button of a panel, the command palette and the agent call
  the same commands ([commands.md](commands.md)); the window draws their state and adds nothing that only a click
  can do.
- **The layout is the user's.** Where the panels are, their size, what is folded, which views are detached: the
  settings of the user, never those of the project, never the history. A project opened on another computer opens
  in that user's layout.
- **One project, one session, one history**, however many windows show it (section 9).
- **Nothing hidden.** Every gesture has its entry in a menu with its key (notation 2.6), every panel its entry in
  `View`, every command its line in the command palette.
- **What is planned is visible.** The themes to come appear where they will be, greyed, marked Planned, each
  leading to its theme in the roadmap or its issue on GitHub, with no date promised (section 6).
- **Readable at a glance.** The diagram keeps its colours (notation 8): whatever draws the panels, the canvas takes
  its colours from the table of tokens of the notation.

## 2. The window

```text
┌─────────────────────────────────────────────────────────────────────────────────────────┐
│ logo  File Edit View Run Help  ↶ ↷ │ Edit Panel Debug Build │ project — search      │ agent ⚙ ? │
├───────────┬───────────────────────────────────────────────────────────────┬─────────────┤
│ Project   │ check_voltage × │ read_voltage ×        Diagram Text Block Panel │ Inspector   │
│ Palette   │                                                                 │             │
│           │                         canvas                                  │             │
│           ├───────────────────────────────────────────────────────────────┤             │
│           │ Problems │ Run                                                   │             │
├───────────┴───────────────────────────────────────────────────────────────┴─────────────┤
│ file · saved │ last step and its author                            │ color by type │ zoom │
└─────────────────────────────────────────────────────────────────────────────────────────┘
```

- **The top bar is the title bar**: the logo; a house that opens the start page; the menus `File`, `Edit`, `View`, `Run`, `Help`; undo and redo; the
  workspaces (section 6); in the middle, the name of the project and the field that opens the command palette
  (section 5); on the right, the state of the agent (section 8), the settings, the help, the version, the buttons of the
  interface (a sun or a moon that switches to the light or the dark theme, as in the other Craft apps, then the mode
  “Every function in its window” of section 9) and, where the desktop draws none, the buttons of the window. An empty part of the bar moves the window, a double click
  maximizes it. On macOS the menus are those of the system bar, and the bar leaves room for the buttons of the
  system; elsewhere they open from the bar. One description of the menus, as data, gives both (`craft-menu`). Who
  draws the frame: on Windows the IDE, as the other Craft apps and VS Code do (its buttons at the right end of the
  bar, its edges resize the window); on macOS the system, its title bar see-through over the top bar; elsewhere
  `craft-window` decides. `LABCRAFT_TITLEBAR=system` keeps the frame of the system, `=app` asks for the IDE's.
- **Left: what the user takes.** `Project` (the tree of the folder) and `Palette` (the functions to place), two tabs
  of one dock.
- **Centre: the tabs of the functions**, each with its views (section 3).
- **Right: what is selected.** `Inspector` (section 4).
- **Bottom: what happens.** `Problems` and `Run`.
- **The status bar**: the file shown and whether it is written; the last step of the history and its author
  ([commands.md](commands.md) 9.1); in the `Text` view, the line and the column; color by type
  (notation 4.3); the zoom.

Every panel can be moved, docked elsewhere, folded or closed (`craft-ui`, docking); `View > Reset layout` brings back
the layout above.

**The look of the window** (its bars, panels, tabs and widgets; not the canvas, whose colours are the notation's, 8):
colour tokens, radii, spacing and type that every widget reads, in a light and a dark theme. Two looks: **Studio**,
neutral graphite with an indigo accent, the default; **Lab**, blue slate with a teal accent, a setting of the user
(`View > Lab look`) *(proposed)*. They are only defaults: the user changes them.

**The `Settings` dialog**, opened by the gear of the top bar, `File > Settings` and the start page, is modal over the
window (decision of the owner, 2026-11-20). It shows the keys of [commands.md](commands.md) 9.7, one section per group
(`Appearance`, `View`, `Editor`, `Run`), and a field that keeps the keys whose name or help holds what is typed. A
switch at its top chooses the store it writes, `User` or `Project` (with a project open); in `Project`, a key that a
project may not hold is greyed, with “User only”. Each key shows its control (a toggle, a choice, a colour), a line of
help, where its value comes from (`project`, `user` or `default`) and `Reset`, which takes it out of the store shown.
`Library colors` lists the libraries that the project can call (those of `Palette`), each with the colour the user
picked, with `Reset`, or “From the code” until they pick one. A change applies at once, by `settings.set`;
`Close` or Escape closes the dialog. The menu items that toggle a setting (`View > Dark theme`, `View > Drag on the empty
canvas pans`, `Run > Record values` and the others) write the same keys, in the store of the user; one whose key the project
holds shows the project's value, greyed, and the dialog changes it. The window opens at 1,280 × 800 at least *(proposed)*; under 1,000 pixels of width, the side
panels fold first.

## 3. The tabs of a function

A tab shows one function. A double click on a call, on a file of `Project`, or `Open` in the command palette opens it.
A single click on a file opens it as a **preview**: its title in italics, replaced by the next preview, kept when the
user edits it or double-clicks its title. Preview tabs are a setting of the user, on by default.

The **views** of a tab are the parts of its file, each its own way:

| View | Shows and edits | M1 |
|---|---|---|
| `Diagram` | The body of the function and its `layout` (the canvas, notation 2) | Yes, the default |
| `Text` | The whole file (section 7) | Yes |
| `Block` | What a caller sees: the `terminals`, `outline` and `icon` sections and the band (spec 11.1 to 11.4) | Yes: the block enlarged and at its real size, its terminals and its edges dragged, its outline, band, size and colours chosen beside it. Drawing the icon by hand comes later |
| `Panel` | The front panel of the function (D-06) | Planned (M3) |

**Split** shows two views of one function side by side, each with its own scroll: `Text` and `Diagram`, `Text` and
`Block`. A change in one is in the other as soon as the text reads (commands.md 9.1). `Text` with `Block` is how a
developer writes an icon or an outline in M1: the block redraws at real size and enlarged while they type, with the
warnings of `check` under the text.

**The view `Block`** (decisions of the owner, 2026-11-20) shows the block of the function as a call to it from another
library draws it, each input unwired and a variadic parameter with one value: enlarged, at the largest whole zoom up to 8
that its room holds; under it, at its real size in the light theme and in the dark one, side by side; then the warnings of
`check` on the block (spec 11.1 to 11.4), of its file and of the manifest of its library. On the enlarged block, a drag on
a terminal moves it onto a free place of a side, as Alt+drag does on a call (notation 2.4), and a drag of the right edge,
the bottom edge or their corner changes `top`, `left` or both, a place at a time, and stops at the least size that holds
its terminals: every line of the section still applies, and the left and right sides hold the terminals that no line
places, which a smaller block would grow back to hold (spec 11.2). Escape during a drag puts it back. Beside it:

- the outline: the one of the mask of its library, or the ordinary block without one; the ordinary block under a mask
  that gives an outline (`template block`); the D (`template d`); the D of the primitives, Primitive (`template
  primitive`). An outline drawn by its points is shown, and changed in
  `Text`;
- the band: the mask's, `hidden`, `inside` or `above`; none for a function of the root library or of a library of
  primitives (spec 11.2);
- `top` and `left`, a field each, and "Auto", which removes the size;
- `color`, `fill` and `stroke`: a swatch that opens a picker, written when the picker closes; "None" removes the line,
  "Follow the mask" when the mask gives that colour;
- "Reset the block" (`block.reset`).

A value that the function does not write and the mask of its library gives is marked, with the name of the library.
Each change is one command of [commands.md](commands.md) 9.4, one step of undo: the view has no command of its own. A
function that the project does not hold, or an `extern`, is shown read only, with a line that says why.

**A function that no longer draws** (decisions of the owner, 2026-11-06 and 2026-11-24): when its file holds an error
that the drawing cannot get past (a value that nothing names, a name that a library takes…), the view `Diagram` keeps
its last drawing ([architecture.md](architecture.md), the window), dimmed and read only, as the diagram of a text that
does not parse ([commands.md](commands.md) 9.1): it is no longer the file, so what acts on it does not edit the function,
neither a gesture on the canvas nor a command of a menu, a key or the inspector on what it shows; a command that names the
file, typed in the command palette or sent by an agent, still edits it, as any file with errors ([commands.md](commands.md)
9.2). A band across the top of the canvas, over the
drawing, tinted with the colour of errors and a line of that colour under it, says "Not drawn since the last change",
then the code, the place and the message of the first error of the file; "Show in Problems", at its end, brings the panel
`Problems` forward. The band goes, and the drawing comes back, as soon as the function draws again. A change of
appearance (`appearance.block_fill`, `appearance.color_by_type`, the colours of the libraries, [commands.md](commands.md)
9.7) reaches the drawing kept: it is drawn again in it, from the folder as it was when the function last drew (decision
of the owner, 2026-11-25). A function that has
never drawn has no drawing to keep: its frame shows its name and lists its diagnostics instead.

The view of each tab, and its split, are view state (spec 11): kept for the user, never in the file.

**The toolbar of a tab**, one thin line above its views: the views first, as a group of icons of which one is on (a
mode, the name of each on hover); then, apart, the run of the function: `Run` (F5), which runs it with `run.start` and
follows the run to its end, and `Stop` (Shift+F5), `run.abort`; one run at a time, as the session has (commands.md 4).
`Run` gives no arguments yet: it is greyed for a function with an input that has no default, its hover naming the
input (`LC8103`, cli.md 6.6). The `Run` panel says the run going, how it ended and its outputs. `Record values`, a toggle kept in the settings of the
user and off by default, makes a run write its trace (`trace: file`, commands.md 6.4), whose values show on the wires
(notation 6.1); off, a run writes none. `Pause`, `Breakpoint` and `Watch` follow, planned (M5, section 6).
Then the actions on the function shown: `Fit` in M1 step 1, then arranging, detaching (section 9) and the tools of
editing as their steps bring them. Then the `Inspector` toggle, the same in a tab and in a detached window: in a tab it
shows or hides the `Inspector` panel of the window of the project (`view.toggle_panel`), in a detached window its own
inspector (section 9). After a run that wrote no trace, the `Run` panel says so, and says that a trace still shown, with
the values on the wires, is of an earlier run. The zoom stays in the status bar. On the web, where runs cannot go (D-22), `Run` is
greyed and says why.

## 4. The panels

- **`Project`**: the tree of the folder, the files of each library under it, each line with its icon (a folder, a
  file of code, a function with the icon of the view `Diagram`); a function that does not draw shows the number of
  its errors, its icon in the colour of its text; the shipped libraries (`core`, `visa`, `scpi`…), read only, at the end. A file opened
  in a detached window says so. The types of a folder follow its functions, a line each (`type Step`), its file on
  hover. Right click: new function, new type, rename, delete, open in a window, each a command. "New function" opens a
  field in place in the tree, in the folder clicked; Enter writes the function (`function.create`) and opens it. "New
  type" opens a field for its name, then one of several lines for its definition (`struct { … }`, `enum { … }`, or a type
  of the language); Ctrl+Enter or "Create" writes it (`type.create`), and "Pick a type…" makes it an alias of a type of
  the list of notation 2.4. "Rename", or F2 on a line, opens a field in place on it: Enter renames the function or the
  type and its file (`function.rename`, `type.rename`), and a tab or a window that shows the function follows it
  (decisions of the owner, 2026-11-19). A type opens in no tab before the view `Text` (step 7 of M1).
- **`Palette`**: the functions the project can call, grouped by library, with a search field on top that searches as
  quick insertion does (notation 2.6). A function dragged onto the canvas is a node added there (`node.add`).
- **`Inspector`**: what the selection is (decision of the owner, 2026-11-14):
  - one node: its function, its signature, its documentation, the help of each parameter, its values of the last run,
    whether it is fixed, with "Remove the fixed position";
  - one wire: its type, its source and its destination, its value of the last run, its `via` line;
  - one terminal of the frame: its name, its type, its default, each a field (`signature.rename`,
    `signature.set_type`, `signature.set_default`), whether it is fixed and its height on its edge (`terminal.move`),
    with "Remove the fixed position" when it is, and "Remove" (`signature.remove`);
  - nothing: the function itself, with its signature, its documentation, its icon and its `frame size`; its
    documentation is a field (`function.set_doc`), and "Add input" and "Add output" under the signature take a name and,
    for an input, a default, then the type from the list of types of notation 2.4 (`signature.add`); the type of a
    terminal is chosen from that list too (decisions of the owner, 2026-11-19);
  - several: how many, and the actions they share.

  A field changed in the inspector calls the command of the same gesture, and is one step of the history.
- **`Problems`**: the diagnostics of the project, by file, error first; a click opens the function and selects the
  node, or the line in the `Text` view. A filter by severity, and one for the function shown.
- **`Run`**: the run of the function shown, its status, its report and its trace (the bar of the viewer of M0v, as a
  panel), and the choice of the simulation file: a file of the project or of `<labcraft>/`, No simulation, or, from
  0.5.0, **Real hardware**, which runs without a simulation file and in real time, so that real names reach the
  hardware ([io.md](io.md) 3 and 9). From 0.5.0 it has a tab `Console`, with the lines the run writes with
  `console.write_line`, as they come, each with its time; a new run of the function starts it again ([io.md](io.md) 5).

## 5. The command palette

Ctrl+Shift+P, or F1, or a click on the field of the top bar (notation 2.6). It lists every command of the IDE, by its
label, with its key, searched as quick insertion searches; Enter runs it, and a command that needs parameters opens
them in a small form. Typed as an id followed by a JSON object, `node.add {"call": "core.ge"}`, it runs that command
with those parameters, and a badge says whether the object is valid before Enter. The commands that the user cannot
run now are listed, greyed, with the reason; the planned entries (section 6) are listed too.

## 6. Workspaces and what is planned

The **workspaces** of the top bar are the ways of working on the same project, history and selection; changing the
workspace changes the panels and the views offered, nothing else. They show as a group of buttons, as the views of a
tab: the current one on, the planned ones greyed, the hover of each says what it is; in a bar too narrow for them, a
selector with the list on a click. Each entry has a name, a state, one line on what
it does and, when it is planned, a link:

| Workspace | State in M1 | What it is |
|---|---|---|
| `Edit` | Current | Draw, wire and run functions; text and block side by side |
| `Panel` | Planned (M3) | The front panel: controls and indicators of a function |
| `Debug` | Planned (M5) | Probes, breakpoints, step by step on the diagram |
| `Build` | Planned (M6) | A standalone executable, a library with a C ABI, a .NET facade |

Planned entries are also where they will be: `Run > Real hardware` (0.5.0, M4, [io.md](io.md) 9: real instruments and
devices instead of the simulator; they are resources that a program touches, not a target: "target" is kept for `Build`
and deployment), `Run > Build` (M6), `Pause`,
`Breakpoint` and `Watch` in the toolbar of a tab and the `Run` menu (M5), the `Panel` view of a tab (M3), the icon drawn
by hand in `Block`. A planned entry can never be chosen: a click opens a short card that says what it will
be and links to its theme in the roadmap or its issue, in the browser. The entries are **data**, a list shipped with
the IDE (id, label, line, state, milestone: its theme, link), not code per entry; an entry becomes Available in the release that
ships it.

## 7. The text of a function

The `Text` view (the `Text` panel of commands.md 9.1 and of notation 2.6) edits the file as the developer writes it; when the text is written, and what happens to a text that
does not read, is in [commands.md](commands.md) 9.1. In M1 it has: the colours of the tokens of the language, from the
lexer of LabCraft itself, so that it never disagrees with `check`; the diagnostics underlined, with their message on
hover and in `Problems`; completion of names (functions, values, types, keywords of the section it is in) in a popup,
opened by typing or by Ctrl+Space, taken with Enter or Tab; the name under the pointer at rest (the signature of a
function, the type of a value) on hover; "Go to definition" (F12, or Ctrl+click, Cmd+click on a Mac), to the line that binds a
value or to the tab of a function of the project (a type, whose file has no tab yet, is named with its file); "Format file" (Shift+Alt+F, `file.format`) once the text typed is written; one
cursor; the keys of a text field (notation 2.6). Between two checks, the diagnostics underlined follow what is typed,
and the error of a text that does not parse is underlined and in `Problems` too.
Multiple cursors, folding and search come later.

It is a component of its own (section 11): a core with no egui (the text, the selection, the undo of the field,
and two traits, one that colours a line and one, the language service, that gives diagnostics, completions, hover and
definitions, in the types of `lsp-types`), and a thin layer of egui that paints it. Another IDE gives it its own
lexer and its own language service. Not chosen: the editor of VS Code (Monaco) or CodeMirror over the canvas, which
would add a second renderer, with its own focus, keys, fonts and popups, a web view on the desktop, and the
same work again in each IDE; tree-sitter, which would need a second grammar of the language beside the parser.

## 8. The agent attached to the IDE

The channel is that of [mcp.md](mcp.md) 1: on `127.0.0.1` only, a token for each launch in a file only the user can
read, a file that gives the port, so that `labcraft-cli mcp` finds the open window. It is open while the window is; with
the switch of AI off it refuses every agent. With several projects open, each has its channel. `labcraft-cli mcp` attaches to the
window whose project is its project, `--project DIR` or the current folder (mcp.md 1); when no window shows that
project, it starts headless, as in M0b, so that an agent never writes into a project other than the one it was started
on (decision of the owner, 2026-11-15).

- **The first connection of a client asks** (the one exception to D-18, decided by the owner on 2026-11-15). The
  window names the client and the project, and offers three policies, by the `effects` of each command
  (commands.md 1): **Ask** (a command with no effect runs; one with `files` or `run` waits for the user's answer),
  **Edits** (`files` runs; `run` asks), **All** (`files` and `run` run, on the simulator). `history.undo` and
  `history.redo` count as `files`. A question waits 60 s *(proposed)*; with no answer, the command is
  `agent.not_allowed`, and so is a connection refused or not answered. No policy reaches a real instrument: from 0.5.0,
  a run of the agent that opens a real endpoint or a file outside the project waits, and the window asks, endpoint by
  endpoint or folder by folder, For this run, Until the window closes, or Refuse; refused or not answered in 60 s
  *(proposed)*, the open gets `io.not_allowed`, an error of the run, not of the command ([io.md](io.md) 4). Those answers
  are never kept. The answer about the policy is
  kept, if the user wants, for that client and that project, in the settings of the user, never of the project
  (`ai.policies`, commands.md 9.7), keyed by the name the client gives (`clientInfo.name`), so that it answers the
  next connection of that client to that project without asking. The window asks when the client gives its name
  (`initialize`, which is answered at once); a call that comes before the answer waits for it, as long as the agent
  waits, and a question unanswered in time goes and is a refusal. A call on a run that the server answers at once
  (`run.status`, `run.abort`, `run.set_element`) never waits for the user: it runs on the agent's own run, or under
  **All**; otherwise it is refused. Refused, or not allowed by its policy, a command of the agent is
  `agent.not_allowed` (commands.md 3).
- **Its policy is the user's.** The settings of AI and of the agents (the switch, each policy) are refused to an
  agent, whatever its policy (`agent.not_allowed`); `settings.set` of another key is an edit.
- **What it sees.** Everything a command reads, and the state of the window: the function shown in each window, the
  view of each tab, the selection (`view.get_state`), so that "this node" means something. It never changes what the user
  sees: it does not select, scroll, zoom or switch tabs, and the `view.*` commands that would are refused to it
  (mcp.md 2).
- **What the user sees of it.** The pill of the top bar: no agent, or which one is connected, and its policy, or that
  its edit waits for the end of the user's gesture; a click opens its menu, which disconnects it or changes its policy
  for this connection, and keeps that policy for that client and that project when the user asks (decision of the
  owner, 2026-11-21). The menu calls the commands of the window `agents.disconnect` and `agents.set_policy`
  ([commands.md](commands.md) 6.7), which the command palette and a key call as well, and an agent never. Its edits go into the one history, signed with its name (commands.md 9.1, 9.5); the nodes and
  wires it touched are highlighted for 2 s *(proposed)*: a node new, changed or moved, a wire new or whose waypoints
  moved; an edit in a function that is not shown puts a dot on its tab, until it is shown; a short notice says what
  it did, for 8 s *(proposed)*, with Show, which shows the function and highlights again what it touched there if a
  window showed it, and Undo, which undoes that step while it is the last one.
- **One switch for AI**, in the settings (`ai.enabled`, commands.md 9.7), on by default: off, the agents attached are
  disconnected, and the window takes every trace of AI out of it, the pill and the notices included. The window still
  answers on its channel, to refuse: a `labcraft-cli mcp` started on its project then stops and says why, rather than
  edit the project from a session of its own (decision of the owner, 2026-11-21). The policies kept are listed under
  it, each with what removes it.
- **Connect an AI agent**, in `Help` and on the start page, writes the entry of the server in the configuration of
  the MCP clients found on the computer, changing nothing else in those files, as `labcraft-cli mcp setup` does with
  the `labcraft-cli` installed beside the IDE ([mcp.md](mcp.md) 9). A dialog then says, a line for each client, what
  it did, and that the clients written must be restarted. It writes only when the user asks, never at a launch
  (decision of the owner, 2026-11-22). With the switch of AI off, it is not there.
- **Its files.** An agent reads and writes only the project of its window, as `labcraft-cli mcp --project DIR`.

A chat panel inside the window, and a visible pointer of the agent, are left for later (section 13).

## 9. Windows

- **One project per window.** `Open folder` and `New project` from a window that shows a project open another window.
  The windows of one project share its session.
- **Detached windows** (decision of the owner, 2026-11-15). A tab of a function can be detached into a window of the
  system, and docked back. That window has the views of its function, its own canvas and its own inspector, for its
  own selection; the menus stay in the window of the project, which keeps `Project`, `Palette`, `Problems` and `Run`.
  Ctrl+Z, in any window of the project, undoes the last step of the project. A layout where the window of the
  project holds only the panels and every function has its own window is a layout like the others, kept by the user.
- **A detached window, in use** (decisions of the owner, 2026-11-16). It holds one function: a tab dropped on it is
  refused. Its toolbar has the views, `Fit`, `Dock`, and two toggles: its inspector, on the right and shown until the
  user hides it, and the palette of functions, on the left and hidden until the user shows it. A tab is detached by
  `Detach` in its toolbar or its menu, by the command palette, or by dragging it out of the window, and docked again by
  `Dock`, by dragging it back, or by `view.reset_layout`; closing the window closes the tab. A detached window opens
  where the last one was, a second screen included. The window that has focus is the one the keys, the command palette
  and the commands that take the function shown act on (`Fit`, a view, Ctrl+W, a trace opened by hand); Ctrl+Z still
  undoes the last step of the project. `Problems` and `Run` follow that window; the inspector of the window of the
  project stays on its function. Each window shows the values of the last run of its function.
- **Every function in its window**, a mode that detaches every tab, the new ones too, is planned: shown in the top bar,
  beside the button of the theme, since it changes the interface and not one function, and in `View`, with its card, until the windows of the system that `craft-ui` opens have their place in the
  taskbar, minimize and maximize (craft-libs#1903).
- On the web there is one window of the browser: a detached tab becomes a panel floating in it.

## 10. Start, new and open

- **The start page** opens when the IDE starts with no folder, and from the house of the top bar: the recent projects
  (12 at most, *(proposed)*, a project that is no longer there marked "not found", the check made off the window
  thread), `New project`, `Open folder`, the examples, and the settings, the keys and `Connect an AI agent`.
- **`New project`** asks for a folder, makes it with a function `main` (`main.lcf`, `fn main() -> () {}`), and opens
  it with `main` shown. The root of a project needs no manifest (spec 1).
- **`Open folder`** opens any folder of `.lcf` files: the project is the folder (mcp.md 1).
- **A file opened from the system** (a double click on a `.lcf` or a `.lct` in the Explorer of Windows, or
  `labcraft FILE`) opens the nearest folder above it that a session opened once (it holds `.labcraft/`), else its own
  folder, and shows it in a tab that stays; a window that shows that project already shows it, and comes to the front
  ([cli.md](cli.md) 8).
- **An example** opens as a copy, in a folder the user chooses, never the files shipped with LabCraft; on the web, in
  memory (section 12).

## 11. Pieces for other IDEs

LabCraft will not be the only IDE of its kind. The pieces of the window that are not about LabCraft are crates of
their own in this repository, which depend on no crate of LabCraft (a rule that `cargo xtask layers` gains with
them), with their own
documentation, MIT OR Apache-2.0; when they are stable, they are offered to `craft-libs`:

- the text component (section 7): `kit-text`, with no UI toolkit, and `kit-text-egui`, which paints it;
- the shell of the window, `kit-shell`: the top bar, the workspaces with their planned entries, the start page,
  the command palette and its JSON form, the inspector as a list of fields;
- the docking and the detached windows come from `craft-ui` and `craft-window`; the keys from `craft-keymap`, the
  menus from `craft-menu`, the settings from `craft-settings`.

The canvas stays LabCraft's (`editor`), embeddable in another Craft app (D-27). Whether these pieces become a UI kit of
their own is decided when a second IDE starts.

## 12. The web

The same window, written once for the desktop and the browser, replaces the two panels of the viewer of M0v. In M1 the
web page edits a project held in memory (the examples, or a folder dropped on it), which `Download` saves as a zip;
it has no agent attached, and a detached tab floats in the page (9). The settings of the user are kept in the
`localStorage` of the page, those of the project in its `.labcraft/config.toml`, in memory and in the zip. Opening a
folder of the computer to write it in place (the File System Access API, which only Chromium has) is not in M1
([architecture.md](architecture.md), 12.9).

Its start page holds the project in memory (decision of the owner, 2026-11-16): an example opens as it is, with no
copy and nothing written on a disk; `New project` makes `main` in memory; `Open folder` chooses a folder, or one is
dropped on the page; no project is recent. The page holds one project: another replaces it, and when the project
changed (a file written, or a step in its history) the page asks first, as it does before it is left or reloaded. The
address names the folder and the function shown.

## 13. Left for later

- Drawing the icon by hand in `Block`, with a throwaway HTML prototype first.
- A chat with the agent inside the window, and a visible pointer of the agent that acts out its commands.
- Multiple cursors, folding and search in the text; a language server for the editors of others, built on the same
  language service.
- Named layouts, and a layout per workspace.
