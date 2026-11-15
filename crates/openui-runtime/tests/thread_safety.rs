//! Prepared inputs and the store retain main's cross-thread guarantees.
use openui_lang::PreparedInput;
use openui_runtime::{PreparedReferenceValues, Store};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
#[test]
fn prepared_inputs_and_store_can_be_sent_and_shared() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<PreparedReferenceValues<'static>>();
    send_sync::<PreparedInput>();
    send_sync::<Store>();
}
