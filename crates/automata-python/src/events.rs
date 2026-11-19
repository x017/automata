//! Python-level events: named messages with arbitrary Python payloads,
//! carried by the engine's double-buffered event bus.
//!
//! Events sent during a frame are readable in that same frame and the
//! following one, then they expire — see `automata_scene::Events`.

use std::cell::RefCell;
use std::rc::Rc;

use automata_scene::Events;
use pyo3::prelude::*;

/// The engine-side event record: a name plus a Python payload.
#[derive(Debug)]
pub struct PyEvent {
    /// The event name.
    pub name: String,
    /// The payload object.
    pub payload: Py<PyAny>,
}

/// Event access during the update callback.
///
/// `read` observes events sent in earlier frames. `send` buffers the event
/// and publishes it right after the callback returns, so it becomes
/// readable in the *next* frame and expires after the frame that follows.
#[pyclass(name = "Events", unsendable, module = "automata._core")]
#[derive(Debug, Clone)]
pub struct PyEvents {
    /// `(name, payload)` pairs readable this frame.
    readable: Rc<Vec<(String, Py<PyAny>)>>,
    /// Buffered outgoing events.
    outbox: EventOutbox,
}

/// Shared outbox between the view and the game loop.
pub type EventOutbox = Rc<RefCell<Vec<PyEvent>>>;

impl PyEvents {
    /// Snapshots the readable events and binds a fresh outbox.
    pub fn snapshot(events: &Events, outbox: EventOutbox) -> Self {
        let readable = Rc::new(
            events
                .read::<PyEvent>()
                .map(|event| (event.name.clone(), clone_payload(&event.payload)))
                .collect(),
        );
        Self { readable, outbox }
    }

    /// Drains the outbox into the engine's event bus.
    pub fn flush(events: &mut Events, outbox: &EventOutbox) {
        for event in outbox.borrow_mut().drain(..) {
            events.send(event);
        }
    }
}

/// Clones a payload reference. Payloads are cloned inside the frame
/// callback where the GIL is held, so plain `clone_ref` is unavailable;
/// `Python::attach` is cheap in that context.
fn clone_payload(payload: &Py<PyAny>) -> Py<PyAny> {
    Python::attach(|py| payload.clone_ref(py))
}

#[pymethods]
impl PyEvents {
    /// Sends a named event with an arbitrary payload (default `None`).
    #[pyo3(signature = (name, payload=None))]
    fn send(&self, py: Python<'_>, name: &str, payload: Option<Py<PyAny>>) {
        let payload = payload.unwrap_or_else(|| py.None());
        self.outbox.borrow_mut().push(PyEvent {
            name: name.to_owned(),
            payload,
        });
    }

    /// The payloads of all readable events with the given name, oldest
    /// first.
    fn read(&self, py: Python<'_>, name: &str) -> Vec<Py<PyAny>> {
        self.readable
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, payload)| payload.clone_ref(py))
            .collect()
    }
}
