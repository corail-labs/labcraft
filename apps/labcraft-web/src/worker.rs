//! The Web Worker of the session (`docs/architecture.md`, 11.5): each message of the page goes to [`Host`], and its answer
//! goes back, in `postcard`. It says first that it listens, and names each function just before it draws it, so that the
//! page puts a trap of WASM down to that drawing. A bug that panics traps the worker, which the page sees and restarts.

use std::cell::RefCell;

use js_sys::Uint8Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent};

use crate::host::Host;
use crate::protocol::{Answer, Failure, FromWorker, State, ToWorker, decode, encode};

thread_local! {
    /// The scope of the worker, for [`announce`], which `engine` calls as a plain function.
    static SCOPE: RefCell<Option<DedicatedWorkerGlobalScope>> = const { RefCell::new(None) };
}

/// Listens to the page, for as long as the worker lives.
pub fn start() {
    // What panicked goes to the console of the browser: the page only sees that the worker stopped.
    std::panic::set_hook(Box::new(|info| web_sys::console::error_1(&format!("labcraft worker: {info}").into())));
    let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    SCOPE.with_borrow_mut(|kept| *kept = Some(scope.clone()));
    let mut host = Host::new(Some(announce));
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let bytes = Uint8Array::new(&event.data()).to_vec();
        let message = decode::<ToWorker>(&bytes);
        // The folder of an `Open` is held once more while it is taken, not twice.
        drop(bytes);
        let answer = match message {
            Ok(ToWorker::Trap) => core::arch::wasm32::unreachable(),
            Ok(message) => host.handle(message),
            Err(message) => Answer { result: Err(Failure { code: "params.invalid".to_owned(), message }), state: State::default(), drawing: None },
        };
        post(&FromWorker::Answer(answer));
    });
    scope.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    // The worker listens until the page ends it.
    on_message.forget();
    post(&FromWorker::Ready);
}

/// Tells the page which function is drawn now.
fn announce(function: &str) {
    post(&FromWorker::Drawing(function.to_owned()));
}

fn post(message: &FromWorker) {
    let bytes = match encode(message) {
        Ok(bytes) => bytes,
        Err(message) => {
            let failed = Answer { result: Err(Failure { code: "internal".to_owned(), message }), state: State::default(), drawing: None };
            encode(&FromWorker::Answer(failed)).unwrap_or_default()
        }
    };
    let array = Uint8Array::from(bytes.as_slice());
    SCOPE.with_borrow(|scope| {
        if let Some(scope) = scope {
            let _ = scope.post_message_with_transfer(&array, &js_sys::Array::of1(&array.buffer()));
        }
    });
}
