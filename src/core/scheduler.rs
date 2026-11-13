use crate::core::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakKind {
    Short,
    Long,
}

impl BreakKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Short => "Short Break",
            Self::Long => "Long Break",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreakPlan {
    pub kind: BreakKind,
    pub duration_seconds: u64,
    pub prompt: String,
}

/// Deterministic break scheduler that tracks short/long break cadence and exercise rotation.
#[derive(Debug, Clone)]
pub struct BreakScheduler {
    short_breaks_completed: u32,
    short_exercise_idx: usize,
    long_exercise_idx: usize,
}

impl BreakScheduler {
    pub fn new() -> Self {
        Self {
            short_breaks_completed: 0,
            short_exercise_idx: 0,
            long_exercise_idx: 0,
        }
    }

    /// Peeks what kind of break is next without advancing counters.
    pub fn next_break_kind(&self, config: &Config) -> BreakKind {
        if config.short_breaks_before_long > 0
            && self.short_breaks_completed >= config.short_breaks_before_long
        {
            BreakKind::Long
        } else {
            BreakKind::Short
        }
    }

    /// Builds the next break plan and advances the exercise/cycle counters.
    pub fn take_next_break(&mut self, config: &Config) -> BreakPlan {
        let kind = self.next_break_kind(config);
        match kind {
            BreakKind::Short => {
                self.short_breaks_completed = self.short_breaks_completed.saturating_add(1);
                let prompt = pick_prompt(
                    &config.short_break_exercises,
                    &mut self.short_exercise_idx,
                    "Look away from the screen and blink slowly",
                );
                BreakPlan {
                    kind,
                    duration_seconds: config.short_break_duration_seconds,
                    prompt,
                }
            }
            BreakKind::Long => {
                self.short_breaks_completed = 0;
                let prompt = pick_prompt(
                    &config.long_break_exercises,
                    &mut self.long_exercise_idx,
                    "Stand up, stretch, and rest your eyes",
                );
                BreakPlan {
                    kind,
                    duration_seconds: config.long_break_duration_seconds,
                    prompt,
                }
            }
        }
    }

    /// Forces an immediate break plan of the requested kind without corrupting long-break cadence.
    pub fn take_manual_break(&mut self, config: &Config, kind: BreakKind) -> BreakPlan {
        match kind {
            BreakKind::Short => {
                let prompt = pick_prompt(
                    &config.short_break_exercises,
                    &mut self.short_exercise_idx,
                    "Look away from the screen and blink slowly",
                );
                BreakPlan {
                    kind,
                    duration_seconds: config.short_break_duration_seconds,
                    prompt,
                }
            }
            BreakKind::Long => {
                self.short_breaks_completed = 0;
                let prompt = pick_prompt(
                    &config.long_break_exercises,
                    &mut self.long_exercise_idx,
                    "Stand up, stretch, and rest your eyes",
                );
                BreakPlan {
                    kind,
                    duration_seconds: config.long_break_duration_seconds,
                    prompt,
                }
            }
        }
    }
}

impl Default for BreakScheduler {
    fn default() -> Self {
        Self::new()
    }
}

fn pick_prompt(exercises: &[String], index: &mut usize, fallback: &str) -> String {
    if exercises.is_empty() {
        return fallback.to_string();
    }
    let idx = *index % exercises.len();
    *index = (idx + 1) % exercises.len();
    exercises[idx].clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternates_short_and_long_breaks_by_config() {
        let cfg = Config {
            short_breaks_before_long: 2,
            short_break_duration_seconds: 15,
            long_break_duration_seconds: 60,
            ..Default::default()
        };
        let mut sched = BreakScheduler::new();

        assert_eq!(sched.take_next_break(&cfg).kind, BreakKind::Short);
        assert_eq!(sched.take_next_break(&cfg).kind, BreakKind::Short);
        let long = sched.take_next_break(&cfg);
        assert_eq!(long.kind, BreakKind::Long);
        assert_eq!(long.duration_seconds, 60);
        assert_eq!(sched.take_next_break(&cfg).kind, BreakKind::Short);
    }
}
