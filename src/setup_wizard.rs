//! First-run setup wizard shown in the GUI (Portuguese).

use std::fs;
use std::path::PathBuf;

use crate::autostart;

/// Number of wizard pages (0-based last index is `PAGE_COUNT - 1`).
pub const PAGE_COUNT: usize = 4;

const MARKER_NAME: &str = ".setup-wizard-complete";

/// In-progress first-run setup choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupWizard {
    /// Current page index (`0..PAGE_COUNT`).
    pub page: usize,
    /// Enable start-with-Windows when the wizard finishes.
    pub autostart: bool,
    /// Start the background watch loop when the wizard finishes.
    pub start_watching: bool,
}

impl Default for SetupWizard {
    fn default() -> Self {
        Self {
            page: 0,
            autostart: true,
            start_watching: true,
        }
    }
}

impl SetupWizard {
    /// Build a wizard seeded with the current autostart toggle.
    pub fn new(autostart: bool, start_watching: bool) -> Self {
        Self {
            page: 0,
            autostart,
            start_watching,
        }
    }

    /// Whether Back is valid on this page.
    pub fn can_go_back(&self) -> bool {
        self.page > 0
    }

    /// Whether this is the last page (Finish instead of Next).
    pub fn is_last(&self) -> bool {
        self.page + 1 >= PAGE_COUNT
    }

    /// Advance one page, clamped to the last page.
    pub fn next(&mut self) {
        if !self.is_last() {
            self.page += 1;
        }
    }

    /// Go back one page, clamped to the first page.
    pub fn back(&mut self) {
        if self.can_go_back() {
            self.page -= 1;
        }
    }

    /// Title and body copy for the current page.
    pub fn copy(&self) -> WizardPageCopy {
        page_copy(self.page)
    }
}

/// Portuguese strings for one wizard page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WizardPageCopy {
    /// Heading.
    pub title: &'static str,
    /// Supporting text.
    pub body: &'static str,
}

/// Copy for a page index (unknown pages fall back to the last page).
pub fn page_copy(page: usize) -> WizardPageCopy {
    match page {
        0 => WizardPageCopy {
            title: "Bem-vindo",
            body: "Vamos configurar o Game Optimizer em poucos passos. \
Seus jogos nunca são fechados — o programa só ajusta prioridade e memória.",
        },
        1 => WizardPageCopy {
            title: "Como funciona",
            body: "Detecta jogos abertos e aplica o contrato de desempenho. \
Minimizar ou fechar a janela envia o app para a bandeja. \
A limpeza da pasta Temp é opcional e nunca apaga cache de jogos.",
        },
        2 => WizardPageCopy {
            title: "Preferências",
            body: "Escolha se o otimizador deve abrir com o Windows e se \
já começa a cuidar dos jogos em segundo plano.",
        },
        _ => WizardPageCopy {
            title: "Tudo pronto",
            body: "Pode otimizar quando quiser. Toque em Concluir para \
abrir o painel. Você pode reabrir este assistente em Mais opções.",
        },
    }
}

/// Path of the first-run completion marker.
pub fn marker_path() -> PathBuf {
    autostart::data_dir().join(MARKER_NAME)
}

/// Whether the first-run wizard has already been completed.
pub fn is_complete() -> bool {
    marker_path().is_file()
}

/// Whether the GUI should show the wizard (`force` re-opens it).
pub fn should_show(force: bool) -> bool {
    force || !is_complete()
}

/// Persist that the wizard finished so it does not open again.
pub fn mark_complete() -> anyhow::Result<()> {
    let path = marker_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, b"1")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_pages_with_last_flag() {
        let mut wizard = SetupWizard::default();
        assert_eq!(PAGE_COUNT, 4);
        assert!(!wizard.is_last());
        assert!(!wizard.can_go_back());
        wizard.next();
        wizard.next();
        wizard.next();
        assert!(wizard.is_last());
        wizard.next();
        assert_eq!(wizard.page, 3);
        wizard.back();
        assert_eq!(wizard.page, 2);
        assert!(wizard.can_go_back());
    }

    #[test]
    fn page_copy_covers_all_indices() {
        assert_eq!(page_copy(0).title, "Bem-vindo");
        assert_eq!(page_copy(1).title, "Como funciona");
        assert_eq!(page_copy(2).title, "Preferências");
        assert_eq!(page_copy(3).title, "Tudo pronto");
        assert_eq!(page_copy(99).title, "Tudo pronto");
    }

    #[test]
    fn defaults_enable_autostart_and_watch() {
        let wizard = SetupWizard::default();
        assert!(wizard.autostart);
        assert!(wizard.start_watching);
    }

    #[test]
    fn should_show_force_ignores_complete_flag() {
        assert!(should_show(true));
    }

    #[test]
    fn marker_lives_under_game_optimizer_dir() {
        let path = marker_path();
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some(".setup-wizard-complete")
        );
    }
}
