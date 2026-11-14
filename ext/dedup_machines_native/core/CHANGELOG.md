# Changelog

## [0.6.0](https://github.com/seuros/dedup_machines/compare/dedup-machines-v0.5.0...dedup-machines-v0.6.0) (2026-11-14)


### ⚠ BREAKING CHANGES

* read claim ids, attempts and entries by exact class, clone-then-keep claims, expire_where asks before dropping

### Features

* read claim ids, attempts and entries by exact class, clone-then-keep claims, expire_where asks before dropping ([90e8ba3](https://github.com/seuros/dedup_machines/commit/90e8ba30efad662f6757033ea49e03cad78c5a7d))

## [0.5.0](https://github.com/seuros/dedup_machines/compare/dedup-machines-v0.4.1...dedup-machines-v0.5.0) (2026-11-13)


### ⚠ BREAKING CHANGES

* converted to the fleet conventions. One exhaustive Refusal per crate, Result from every operation that can refuse, a refusal changes nothing, provable idempotent repeats are honoured, Ruby mirrors the core. No compatibility aliases. Claim holds honoured answers only; ClaimId is a scoped u64; restore is resumed(store, scope, issued, observed); Storage gains locate and iter yields slots. CI now tests the alloc and std features.

### Features

* one Refusal, scoped never-reissued claim ids, lazy expiry and refusals that change nothing ([9e41b46](https://github.com/seuros/dedup_machines/commit/9e41b46ec7ab5264ce4ed7db72ce98a3d6f4785b))

## [0.4.1](https://github.com/seuros/dedup_machines/compare/dedup-machines-v0.4.0...dedup-machines-v0.4.1) (2026-11-06)


### Bug Fixes

* satisfy the nursery option_if_let_else lint in Entry::new ([88cf1eb](https://github.com/seuros/dedup_machines/commit/88cf1ebadcbfac752ac6e20fb0d2ce07dae765e9))
* share core test helpers via test_support ([0c94d57](https://github.com/seuros/dedup_machines/commit/0c94d57d888c41b77d4a11c9db346fc0587ee78c))

## [0.4.0](https://github.com/seuros/dedup_machines/compare/dedup-machines-v0.3.0...dedup-machines-v0.4.0) (2026-11-06)


### Features

* unseal Storage so a ledger can run over an external backend ([61c8c77](https://github.com/seuros/dedup_machines/commit/61c8c77d45923109919a5dda5b1315ce64427aba))

## [0.3.0](https://github.com/seuros/dedup_machines/compare/dedup-machines-v0.2.2...dedup-machines-v0.3.0) (2026-11-06)


### ⚠ BREAKING CHANGES

* Claim carries a lifetime and value type, and Completed holds a reference to the stored value.

### Features

* heap-backed ledger, fingerprints, stored values, partition caps ([a9fee64](https://github.com/seuros/dedup_machines/commit/a9fee64ce94d8a32e62c9d97e014f3664fa8b20e))


### Bug Fixes

* document the error path of complete_with ([0c9dc20](https://github.com/seuros/dedup_machines/commit/0c9dc206c4c8ec55be2d6a37420eb7c4d101fb8b))
* satisfy pedantic clippy on the test helpers and hashed constructor ([001a626](https://github.com/seuros/dedup_machines/commit/001a6265621f8f46a7b1d38e881ce04ead09c40f))

## [0.2.2](https://github.com/seuros/dedup_machines/compare/dedup-machines-v0.2.1...dedup-machines-v0.2.2) (2026-11-06)


### Bug Fixes

* crates.io keywords and categories for the core ([2c5c7ee](https://github.com/seuros/dedup_machines/commit/2c5c7eebc94e34f772bdd7508d12f7d79c334c81))

## [0.2.1](https://github.com/seuros/dedup_machines/compare/dedup-machines-v0.2.0...dedup-machines-v0.2.1) (2026-11-06)


### Bug Fixes

* keep test sources out of the packaged gem and crate ([dc18303](https://github.com/seuros/dedup_machines/commit/dc1830331de56e948e2ce082686be122a8a537aa))

## [0.2.0](https://github.com/seuros/dedup_machines/compare/dedup-machines-v0.1.0...dedup-machines-v0.2.0) (2026-11-06)


### Features

* initial extraction ([126339b](https://github.com/seuros/dedup_machines/commit/126339bf2ec50261af08c5e710436bafa2530365))
