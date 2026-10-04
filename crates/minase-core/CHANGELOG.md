## minase-core-v0.1.1 (2026-10-04)

### Documentation

- Recommend LTO and one codegen unit in dependent crates

  Cargo builds a dependency with the dependent's profile, and with Cargo's default release profile legal-move generation runs at about 61% of its speed with LTO. The README now gives the profile to put in the dependent's own `Cargo.toml`.

- Add crates.io keywords and categories

## minase-core-v0.1.0 (2026-10-04)

### Refactoring

- Split the library into the minase-core crate (**breaking**)

  The rules library moved out of `minase` into this crate. Library types moved from `minase::` to `minase_core::` (for example, `minase::Position` is now `minase_core::Position`), and the former `minase::core::` modules sit directly under `minase_core::`.


