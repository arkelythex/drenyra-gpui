//! Native shell presentation layer.
//!
//! `view_model` and `commands` are GPUI-free and fully unit-tested; the feature-gated `window`
//! module only maps the view model to GPUI elements and key bindings to commands.

pub mod agent_pane;
pub mod commands;
pub mod demo;
pub mod settings;
pub mod theme;
pub mod view_model;

#[cfg(test)]
mod regression_tests;
#[cfg(any(feature = "gpui", test))]
pub mod scroll;
#[cfg(test)]
mod t10_probes;
#[cfg(feature = "gpui")]
pub mod window;
