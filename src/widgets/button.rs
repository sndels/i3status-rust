use serde_json::value::Value;

use super::I3BarWidget;
use super::Spacing;
use super::State;
use crate::config::SharedConfig;

#[derive(Clone, Debug)]
pub struct ButtonWidget {
    id: usize,
    content: Option<String>,
    icon: Option<String>,
    state: State,
    spacing: Spacing,
    rendered: Value,
    cached_output: Option<String>,
    shared_config: SharedConfig,
}

impl ButtonWidget {
    pub fn new(id: usize, shared_config: SharedConfig) -> Self {
        ButtonWidget {
            id,
            content: None,
            icon: None,
            state: State::Idle,
            spacing: Spacing::Normal,
            rendered: json!({
                "full_text": "",
                "separator": false,
                "separator_block_width": 0,
                "background": "#000000",
                "color": "#000000",
                "markup": "pango"
            }),
            cached_output: None,
            shared_config,
        }
    }

    pub fn with_icon(mut self, name: &str) -> Self {
        self.icon = self.shared_config.get_icon(name);
        self.update();
        self
    }

    pub fn with_content(mut self, content: Option<String>) -> Self {
        self.content = content;
        self.update();
        self
    }

    pub fn with_text(mut self, content: &str) -> Self {
        self.content = Some(String::from(content));
        self.update();
        self
    }

    pub fn with_state(mut self, state: State) -> Self {
        self.state = state;
        self.update();
        self
    }

    pub fn with_spacing(mut self, spacing: Spacing) -> Self {
        self.spacing = spacing;
        self.update();
        self
    }

    pub fn set_text<S: Into<String>>(&mut self, content: S) {
        self.content = Some(content.into());
        self.update();
    }

    pub fn set_icon(&mut self, name: &str) {
        self.icon = self.shared_config.get_icon(name);
        self.update();
    }

    pub fn set_state(&mut self, state: State) {
        self.state = state;
        self.update();
    }

    pub fn set_spacing(&mut self, spacing: Spacing) {
        self.spacing = spacing;
        self.update();
    }

    fn update(&mut self) {
        let (key_bg, key_fg) = self.state.theme_keys(&self.shared_config.theme);

        // When rendered inline, remove the leading space
        self.rendered = json!({
            "full_text": format!("{}{}{}",
                                self.icon.clone().unwrap_or_else(|| {
                                    match self.spacing {
                                        Spacing::Normal => String::from(" "),
                                        _ => String::from("")
                                    }
                                }),
                                self.content.clone().unwrap_or_default(),
                                match self.spacing {
                                    Spacing::Hidden => String::from(""),
                                    _ => String::from(" ")
                                }
                            ),
            "separator": false,
            "name": self.id,
            "separator_block_width": 0,
            "background": key_bg,
            "color": key_fg,
            "markup": "pango"
        });

        self.cached_output = Some(self.rendered.to_string());
    }
}

impl I3BarWidget for ButtonWidget {
    fn to_string(&self) -> String {
        self.cached_output
            .clone()
            .unwrap_or_else(|| self.rendered.to_string())
    }

    fn get_rendered(&self) -> &Value {
        &self.rendered
    }
}
