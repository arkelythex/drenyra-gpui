//! Native GPUI host shell descriptor and feature-gated mount boundary.

use drenyra_workspace::{FiscalWorkspace, SessionGuardrails};

/// Declarative configuration for the native GPUI desktop shell window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopShellConfig {
    pub app_title: String,
    pub window_width: u32,
    pub window_height: u32,
    pub rda_banner_visible: bool,
}

impl Default for DesktopShellConfig {
    fn default() -> Self {
        Self {
            app_title: "Drenyra GPUI".into(),
            window_width: 1440,
            window_height: 900,
            rda_banner_visible: SessionGuardrails::default().rda_enabled,
        }
    }
}

/// Initial workspace and shell state passed into the GPUI window mount hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowBootstrapPlan {
    pub config: DesktopShellConfig,
    pub workspace: FiscalWorkspace,
}

impl WindowBootstrapPlan {
    pub fn from_workspace(config: DesktopShellConfig, workspace: FiscalWorkspace) -> Self {
        Self { config, workspace }
    }

    pub fn default_bootstrap() -> Self {
        Self::from_workspace(
            DesktopShellConfig::default(),
            FiscalWorkspace::new("00000000000", "Default Entity", "2026-01"),
        )
    }

    pub fn initial_pane_count(&self) -> usize {
        self.workspace.pane_count()
    }

    pub const fn is_gpui_feature_enabled() -> bool {
        cfg!(feature = "gpui")
    }
}

/// Mount boundary for native GPUI builds (`cargo build --features gpui`).
#[cfg(feature = "gpui")]
pub fn mount_native_window(plan: &WindowBootstrapPlan) -> Result<(), String> {
    if plan.config.window_width == 0 || plan.config.window_height == 0 {
        return Err("window dimensions must be non-zero".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_bootstrap_plan_defaults_preserve_rda_and_single_pane() {
        let plan = WindowBootstrapPlan::default_bootstrap();
        assert_eq!(plan.config.app_title, "Drenyra GPUI");
        assert!(plan.config.rda_banner_visible);
        assert_eq!(plan.initial_pane_count(), 1);
        assert_eq!(
            WindowBootstrapPlan::is_gpui_feature_enabled(),
            cfg!(feature = "gpui")
        );
    }

    #[cfg(feature = "gpui")]
    #[test]
    fn mount_native_window_validates_dimensions() {
        let mut plan = WindowBootstrapPlan::default_bootstrap();
        assert!(mount_native_window(&plan).is_ok());
        plan.config.window_width = 0;
        assert!(mount_native_window(&plan).is_err());
    }
}
