//! The value classes, resolved once at init and pinned. The exact-class
//! reads and clone-then-keep are `matryona_machines::ruby`'s.
use magnus::{Error, RModule, Ruby};
use matryona_machines::ruby::{Class, Registry};

/// The `Data` classes the binding builds and reads.
pub struct Classes {
    pub claim_id: Class,
    pub acquired: Class,
    pub completed: Class,
    pub attempt: Class,
    pub entry: Class,
}

static CLASSES: Registry<Classes> = Registry::new("dedup_machines");

/// Resolves the classes `lib/dedup_machines.rb` defined before it loaded the
/// extension. Called once, from init; never by name again.
pub fn resolve(ruby: &Ruby, module: RModule) -> Result<(), Error> {
    let class = |name| Class::resolve(ruby, module, name);
    let classes = Classes {
        claim_id: class("ClaimId")?,
        acquired: class("Acquired")?,
        completed: class("Completed")?,
        attempt: class("Attempt")?,
        entry: class("Entry")?,
    };
    CLASSES.set(ruby, classes).map(drop)
}

/// The classes resolved at init.
pub fn classes(ruby: &Ruby) -> Result<&'static Classes, Error> {
    CLASSES.get(ruby)
}
