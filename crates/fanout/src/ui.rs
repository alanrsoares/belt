//! Terminal rendering, ANSI formatting, and TUI widgets for `fanout`.

pub use local_common::{
    draw_meter as common_draw_meter, format_duration, strip_ansi, terminal_columns, terminal_rows,
    visible_width, CursorGuard, PALETTE, SPINNER,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Pending,
    Running,
    Passed,
    Failed(i32),
    Cancelled,
    Timeout,
}

impl Outcome {
    pub fn is_settled(&self) -> bool {
        !matches!(self, Self::Pending | Self::Running)
    }

    pub fn is_bad(&self) -> bool {
        matches!(self, Self::Failed(_) | Self::Timeout)
    }
}

pub struct Styles {
    pub enabled: bool,
}

impl Styles {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    pub fn sgr<'a>(&self, code: &'a str) -> &'a str {
        if self.enabled {
            code
        } else {
            ""
        }
    }

    pub fn reset(&self) -> &str {
        self.sgr("\x1b[0m")
    }

    pub fn bold(&self) -> &str {
        self.sgr("\x1b[1m")
    }

    pub fn dim(&self) -> &str {
        self.sgr("\x1b[2m")
    }

    pub fn gray(&self) -> &str {
        self.sgr("\x1b[90m")
    }

    pub fn red(&self) -> &str {
        self.sgr("\x1b[31m")
    }

    pub fn green(&self) -> &str {
        self.sgr("\x1b[32m")
    }

    pub fn yellow(&self) -> &str {
        self.sgr("\x1b[33m")
    }

    pub fn cyan(&self) -> &str {
        self.sgr("\x1b[96m")
    }

    pub fn task_color(&self, idx: usize) -> &str {
        if self.enabled {
            PALETTE[idx % PALETTE.len()]
        } else {
            ""
        }
    }
}

pub fn draw_meter(ratio: f64, width: usize, color: &str, s: &Styles) -> String {
    common_draw_meter(ratio, width, color, s.reset(), s.gray())
}

pub fn phase_icon(phase: Outcome, s: &Styles, frame: usize) -> String {
    match phase {
        Outcome::Pending => format!("{}◌{}", s.gray(), s.reset()),
        Outcome::Running => format!(
            "{}{}{}",
            s.cyan(),
            SPINNER[frame % SPINNER.len()],
            s.reset()
        ),
        Outcome::Passed => format!("{}✔{}", s.green(), s.reset()),
        Outcome::Failed(_) => format!("{}✖{}", s.red(), s.reset()),
        Outcome::Cancelled => format!("{}⊘{}", s.gray(), s.reset()),
        Outcome::Timeout => format!("{}⏱{}", s.yellow(), s.reset()),
    }
}

pub fn phase_text(phase: Outcome, s: &Styles) -> String {
    match phase {
        Outcome::Pending => format!("{}pending  {}", s.gray(), s.reset()),
        Outcome::Running => format!("{}running  {}", s.cyan(), s.reset()),
        Outcome::Passed => format!("{}passed   {}", s.green(), s.reset()),
        Outcome::Failed(_) => format!("{}failed   {}", s.red(), s.reset()),
        Outcome::Cancelled => format!("{}cancelled{}", s.gray(), s.reset()),
        Outcome::Timeout => format!("{}timeout  {}", s.yellow(), s.reset()),
    }
}
