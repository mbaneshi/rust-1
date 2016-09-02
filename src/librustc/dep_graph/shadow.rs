//! The "Shadow Graph" is maintained on the main thread and which
//! tracks each message relating to the dep-graph and applies some
//! sanity checks as they go by. If an error results, it means you get
//! a nice stack-trace telling you precisely what caused the error.
//!
//! NOTE: This is a debugging facility which can potentially have non-trivial
//! runtime impact. Therefore, it is largely compiled out if
//! debug-assertions are not enabled.
//!
//! The basic sanity check, always enabled, is that there is always a
//! task (or ignore) on the stack when you do read/write.
//!
//! Optionally, if you specify RUST_FORBID_DEP_GRAPH_EDGE, you can
//! specify an edge filter to be applied to each edge as it is
//! created.  See `./README.md` for details.

use hir::def_id::DefId;
use std::cell::{BorrowState, RefCell};
use std::env;

use super::DepNode;
use super::thread::DepMessage;
use super::debug::EdgeFilter;

pub struct ShadowGraph {
    // if you push None onto the stack, that corresponds to an Ignore
    stack: RefCell<Vec<Option<DepNode<DefId>>>>,
    forbidden_edge: Option<EdgeFilter>,
}

const ENABLED: bool = cfg!(debug_assertions);

impl ShadowGraph {
    pub fn new() -> Self {
        let forbidden_edge = if !ENABLED {
            None
        } else {
            match env::var("RUST_FORBID_DEP_GRAPH_EDGE") {
                Ok(s) => {
                    match EdgeFilter::new(&s) {
                        Ok(f) => Some(f),
                        Err(err) => bug!("RUST_FORBID_DEP_GRAPH_EDGE invalid: {}", err),
                    }
                }
                Err(_) => None,
            }
        };

        ShadowGraph {
            stack: RefCell::new(vec![]),
            forbidden_edge: forbidden_edge,
        }
    }

    pub fn enqueue(&self, message: &DepMessage) {
        if ENABLED {
            match self.stack.borrow_state() {
                BorrowState::Unused => {}
                _ => {
                    // When we apply edge filters, that invokes the
                    // Debug trait on DefIds, which in turn reads from
                    // various bits of state and creates reads! Ignore
                    // those recursive reads.
                    return;
                }
            }

            let mut stack = self.stack.borrow_mut();
            match *message {
                DepMessage::Read(ref n) => self.check_edge(Some(Some(n)), top(&stack)),
                DepMessage::Write(ref n) => self.check_edge(top(&stack), Some(Some(n))),
                DepMessage::PushTask(ref n) => stack.push(Some(n.clone())),
                DepMessage::PushIgnore => stack.push(None),
                DepMessage::PopTask(_) |
                DepMessage::PopIgnore => {
                    // we could easily check that the stack is
                    // well-formed here, but since we use closures and
                    // RAII accessors, this bug basically never
                    // happens, so it seems not worth the overhead
                    stack.pop();
                }
                DepMessage::Query => (),
            }
        }
    }

    fn check_edge(&self,
                  source: Option<Option<&DepNode<DefId>>>,
                  target: Option<Option<&DepNode<DefId>>>) {
        assert!(ENABLED);
        match (source, target) {
            // cannot happen, one side is always Some(Some(_))
            (None, None) => unreachable!(),

            // nothing on top of the stack
            (None, Some(n)) | (Some(n), None) => bug!("read/write of {:?} but no current task", n),

            // this corresponds to an Ignore being top of the stack
            (Some(None), _) | (_, Some(None)) => (),

            // a task is on top of the stack
            (Some(Some(source)), Some(Some(target))) => {
                if let Some(ref forbidden_edge) = self.forbidden_edge {
                    if forbidden_edge.test(source, target) {
                        bug!("forbidden edge {:?} -> {:?} created", source, target)
                    }
                }
            }
        }
    }
}

// Do a little juggling: we get back a reference to an option at the
// top of the stack, convert it to an optional reference.
fn top<'s>(stack: &'s Vec<Option<DepNode<DefId>>>) -> Option<Option<&'s DepNode<DefId>>> {
    stack.last()
        .map(|n: &'s Option<DepNode<DefId>>| -> Option<&'s DepNode<DefId>> {
            // (*)
            // (*) type annotation just there to clarify what would
            // otherwise be some *really* obscure code
            n.as_ref()
        })
}
