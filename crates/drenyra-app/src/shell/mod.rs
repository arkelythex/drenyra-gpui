//! Native shell presentation layer.
//!
//! `view_model` and `commands` are GPUI-free and fully unit-tested; the feature-gated `window`
//! module only maps the view model to GPUI elements and key bindings to commands.

pub mod commands;
pub mod demo;
pub mod view_model;
