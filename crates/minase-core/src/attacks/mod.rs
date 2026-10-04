//! 駒の利き計算(固定利き・走り利き・獅子系の特殊移動)の前計算と参照。

mod fixed;
mod sliding;
mod tables;

pub(crate) use fixed::{LionLikeProfile, SpecialMovement, movement_profile, movement_profile_data};
pub(crate) use tables::{AttackTables, attack_tables};
