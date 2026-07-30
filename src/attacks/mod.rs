mod fixed;
mod sliding;
mod tables;

pub use fixed::{
    JumpingGeneralProfile, LionLikeProfile, MOVEMENT_PROFILE_COUNT, MovementProfile,
    MovementProfileId, RelativeDelta, RelativeDirection, SlideSpec, SpecialMovement, all_profiles,
    movement_profile, movement_profile_data,
};
pub use sliding::{RayTable, build_ray_table, ray_control, sliding_control, sliding_destinations};
pub use tables::{AttackTables, attack_tables};
