//! Private native entry point.
use crate::Ledger;
use magnus::{Error, Ruby, function, method, prelude::*};

#[magnus::init]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("DedupMachines")?;
    let handle = module.define_class("Claim", ruby.class_object())?;
    handle.undef_default_alloc_func();
    let class = module.define_class("Ledger", ruby.class_object())?;
    class.undef_default_alloc_func();
    class.define_singleton_method("new", function!(Ledger::new, 0))?;
    class.define_method("claim", method!(Ledger::claim, 3))?;
    class.define_method("complete", method!(Ledger::complete, 2))?;
    class.define_method("abandon", method!(Ledger::abandon, 1))?;
    class.define_method("capacity", function!(Ledger::capacity, 0))?;
    Ok(())
}
