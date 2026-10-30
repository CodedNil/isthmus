//! Broad contracts: independent ray oracle, enclosure/transport and lifecycle.
use super::*;
use crate::{
    home,
    render::{Scene, SunSettings, temporal::Accumulation},
};
mod deferred;
mod geometry;
mod lifecycle;
