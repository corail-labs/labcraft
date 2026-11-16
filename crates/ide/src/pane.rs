//! The panes of the dock (ide.md 2 and 4): the panels, the tab of each function, and the start page as a tab. Where
//! they are is the layout of the user (`craft-ui` docking), kept in the settings of the user, never the project's; which
//! functions have a tab is the state of the session, so the tabs of functions are never saved with the layout.

use craft_ui::docking::{Layout, Node, Permissions, Workspace};
use craft_ui::layout::{SplitAxis, SplitSize};
use serde::{Deserialize, Serialize};

/// A pane of the dock.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Pane {
    Project,
    Palette,
    Inspector,
    Problems,
    Run,
    /// The start page, as a tab, opened by the house of the top bar; it gives way to the first function opened there.
    Start,
    /// The tab of a function.
    Function(String),
}

/// The panels that `View` shows and hides (`view.toggle_panel`), by the names of commands.md 6.7.
pub const PANELS: &[(Pane, &str, &str)] = &[
    (Pane::Project, "project", "Project"),
    (Pane::Palette, "palette", "Palette"),
    (Pane::Inspector, "inspector", "Inspector"),
    (Pane::Problems, "problems", "Problems"),
    (Pane::Run, "run", "Run"),
];

impl Pane {
    /// The panel of this name of `view.toggle_panel`.
    pub fn panel(name: &str) -> Option<Pane> {
        PANELS.iter().find(|(_, id, _)| *id == name).map(|(pane, _, _)| pane.clone())
    }

    pub fn title(&self) -> String {
        match self {
            Pane::Start => "Start".to_owned(),
            Pane::Function(function) => function.clone(),
            panel => PANELS.iter().find(|(pane, _, _)| pane == panel).map_or_else(String::new, |(_, _, title)| (*title).to_owned()),
        }
    }

    pub fn function(&self) -> Option<&str> {
        match self {
            Pane::Function(function) => Some(function),
            _ => None,
        }
    }
}

/// Every pane can move, float and close; a panel closed is hidden, and `View` brings it back where it was.
pub fn permissions(_pane: &Pane) -> Permissions {
    Permissions::default()
}

/// The layout of ide.md 2: `Project` and `Palette` on the left, the tabs in the centre over `Problems` and `Run`,
/// `Inspector` on the right.
pub fn default_layout() -> Layout<Pane> {
    let tabs = |panels: Vec<Pane>| Node::Tabs { panels, active: 0 };
    let centre = Node::Split {
        axis: SplitAxis::Vertical,
        size: SplitSize::FixedSecond(180.0),
        first: Box::new(tabs(vec![Pane::Start])),
        second: Box::new(tabs(vec![Pane::Problems, Pane::Run])),
    };
    let right = Node::Split {
        axis: SplitAxis::Horizontal,
        size: SplitSize::FixedSecond(280.0),
        first: Box::new(centre),
        second: Box::new(tabs(vec![Pane::Inspector])),
    };
    let root = Node::Split {
        axis: SplitAxis::Horizontal,
        size: SplitSize::FixedFirst(240.0),
        first: Box::new(tabs(vec![Pane::Project, Pane::Palette])),
        second: Box::new(right),
    };
    Layout { root: Some(root), floating: Vec::new() }
}

pub fn default_workspace() -> Workspace<Pane> {
    Workspace::new(default_layout())
}

/// The layout as it is saved: without the tabs of functions, which the session holds. A workspace that does not read
/// back is the default one.
pub fn to_save(workspace: &Workspace<Pane>) -> Workspace<Pane> {
    let mut saved = workspace.clone();
    let functions: Vec<Pane> = saved.layout.panels().into_iter().filter(|pane| pane.function().is_some()).cloned().collect();
    for pane in functions {
        // The start page takes the place of the tabs, so that the next functions open there.
        if !saved.layout.contains(&Pane::Start) {
            let _ = saved.apply(craft_ui::docking::Action::Open { panel: Pane::Start, anchor: Some(pane.clone()) }, permissions);
        }
        let _ = saved.apply(craft_ui::docking::Action::Close { panel: pane }, permissions);
    }
    saved.hidden.retain(|(pane, _)| pane.function().is_none());
    saved
}

/// A workspace read from the settings, made sound: one that does not validate is the default, and so is one without
/// the panels.
pub fn restored(saved: Option<Workspace<Pane>>) -> Workspace<Pane> {
    let default = default_workspace();
    let Some(mut workspace) = saved else { return default };
    workspace.reconcile(|pane| pane.function().is_none(), permissions, &default);
    if workspace.validate().is_err() { default } else { workspace }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_ui::docking::Action;

    #[test]
    fn the_default_layout_holds_every_panel_once() {
        let layout = default_layout();
        assert_eq!(layout.validate(), Ok(()));
        for (pane, _, _) in PANELS {
            assert!(layout.contains(pane), "{pane:?}");
        }
        assert!(layout.contains(&Pane::Start));
    }

    #[test]
    fn the_tabs_of_functions_are_not_saved() {
        let mut workspace = default_workspace();
        workspace.apply(Action::Open { panel: Pane::Function("main".to_owned()), anchor: Some(Pane::Start) }, permissions).unwrap();
        workspace.apply(Action::Close { panel: Pane::Start }, permissions).unwrap();
        let saved = to_save(&workspace);
        assert!(!saved.layout.contains(&Pane::Function("main".to_owned())));
        assert!(saved.layout.contains(&Pane::Start), "the start page keeps the place of the tabs");
        assert_eq!(saved.validate(), Ok(()));
    }

    #[test]
    fn a_saved_layout_that_does_not_read_is_the_default() {
        let broken =
            Workspace::new(Layout { root: Some(Node::Tabs { panels: vec![Pane::Project, Pane::Project], active: 0 }), floating: Vec::new() });
        assert_eq!(restored(Some(broken)).layout, default_layout());
        assert_eq!(restored(None).layout, default_layout());
    }
}
