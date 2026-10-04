## minase-core-v0.1.0 (2026-10-04)

### Refactoring

- Split the library into the minase-core crate (**breaking**)

  The rules library moved out of `minase` into this crate. Library types moved from `minase::` to `minase_core::` (for example, `minase::Position` is now `minase_core::Position`), and the former `minase::core::` modules sit directly under `minase_core::`.


