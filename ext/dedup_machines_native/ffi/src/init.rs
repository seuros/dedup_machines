//! Private native entry point.
use crate::{CAPACITY, Ledger, convert};
use magnus::{Error, Ruby, function, method, prelude::*};

#[magnus::init]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = ruby.define_module("DedupMachines")?;
    module.const_set("CAPACITY", CAPACITY)?;
    crate::classes::resolve(ruby, module)?;
    // Private hooks the Data values validate through (see lib/).
    module.define_singleton_method("__claim_scope", function!(convert::claim_scope, 1))?;
    module.define_singleton_method("__acquired", function!(convert::check_acquired, 1))?;
    module.define_singleton_method("__completed", function!(convert::check_completed, 1))?;
    module.define_singleton_method("__attempt", function!(convert::check_attempt, 1))?;
    module.define_singleton_method("__entry", function!(convert::check_entry, 1))?;
    let class = module.define_class("Ledger", ruby.class_object())?;
    class.undef_default_alloc_func();
    class.define_singleton_method("new", function!(Ledger::new, 0))?;
    class.define_singleton_method("with_scope", function!(Ledger::with_scope, 1))?;
    class.define_singleton_method("resumed", function!(Ledger::resumed, 4))?;
    class.define_method("with_partition_cap", method!(Ledger::with_partition_cap, 1))?;
    class.define_method("scope", method!(Ledger::scope, 0))?;
    class.define_method("issued", method!(Ledger::issued, 0))?;
    class.define_method("observed", method!(Ledger::observed, 0))?;
    class.define_method("partition_cap", method!(Ledger::partition_cap, 0))?;
    class.define_method("capacity", function!(Ledger::capacity, 0))?;
    class.define_method("entries", method!(Ledger::entries, 0))?;
    class.define_method("claim", method!(Ledger::claim, 3))?;
    class.define_method("claim_with", method!(Ledger::claim_with, 4))?;
    class.define_method("admit", method!(Ledger::admit, 3))?;
    class.define_method("complete", method!(Ledger::complete, 2))?;
    class.define_method("complete_with", method!(Ledger::complete_with, 3))?;
    class.define_method("abandon", method!(Ledger::abandon, 1))?;
    class.define_method("expire_where", method!(Ledger::expire_where, 0))?;
    Ok(())
}
