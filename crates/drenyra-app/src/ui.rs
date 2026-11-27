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

impl DesktopShellConfig {
    /// Rejects configurations the window cannot open with.
    pub fn validate(&self) -> Result<(), String> {
        if self.window_width == 0 || self.window_height == 0 {
            return Err("window dimensions must be non-zero".into());
        }
        Ok(())
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

/// Mount boundary for native GPUI builds (`cargo build --features gpui`): validates the
/// configuration, then blocks in the GPUI event loop until the window closes. Fails if the
/// configuration is invalid or the window cannot be opened.
#[cfg(feature = "gpui")]
pub fn mount_native_window(
    config: DesktopShellConfig,
    state: crate::shell::commands::ShellState,
    settings_path: Option<std::path::PathBuf>,
) -> Result<(), String> {
    config.validate()?;
    crate::shell::window::run(config, state, settings_path)
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

    #[test]
    fn desktop_shell_config_validates_dimensions() {
        let mut config = DesktopShellConfig::default();
        assert_eq!(config.validate(), Ok(()));
        config.window_width = 0;
        assert_eq!(
            config.validate(),
            Err("window dimensions must be non-zero".to_owned())
        );
        config.window_width = 1440;
        config.window_height = 0;
        assert!(config.validate().is_err());
    }
}
