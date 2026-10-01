//! Optional style diagnostics over owned, typed Papyrus facts.

use std::collections::BTreeMap;

use folio_diagnostics::{Diagnostic, Severity};
use folio_hir::{ExpressionFact, ExpressionKind, Script, Statement, Type};

/// Stable rule identity shared by CLI and editor diagnostics.
pub const PREFER_TRUTHY_NONE_CHECK: &str = "papyrus.prefer-truthy-none-check";

/// Effective per-project lint severity. `None` disables a rule.
#[derive(Clone, Debug)]
pub struct LintConfig {
    prefer_truthy_none_check: Option<Severity>,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            prefer_truthy_none_check: Some(Severity::Warning),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    UnknownRule(String),
    InvalidLevel { rule: String, level: String },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownRule(rule) => write!(formatter, "unknown lint rule {rule}"),
            Self::InvalidLevel { rule, level } => {
                write!(formatter, "invalid lint level {level} for {rule}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl LintConfig {
    /// Validates the rule names and levels supplied by the project manifest.
    pub fn from_rules(rules: &BTreeMap<String, String>) -> Result<Self, ConfigError> {
        let mut config = Self::default();
        for (rule, level) in rules {
            if rule != PREFER_TRUTHY_NONE_CHECK {
                return Err(ConfigError::UnknownRule(rule.clone()));
            }
            config.prefer_truthy_none_check = match level.as_str() {
                "off" => None,
                "info" => Some(Severity::Info),
                "warning" => Some(Severity::Warning),
                "error" => Some(Severity::Error),
                _ => {
                    return Err(ConfigError::InvalidLevel {
                        rule: rule.clone(),
                        level: level.clone(),
                    });
                }
            };
        }
        Ok(config)
    }
}

/// Applies the enabled rules without needing successful code generation.
#[tracing::instrument(skip(script, config), fields(bodies = script.bodies.len()))]
pub fn lint_script(script: &Script, config: &LintConfig) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for body in &script.bodies {
        for statement in &body.statements {
            visit_statement(statement, config, &mut diagnostics);
        }
    }
    diagnostics.sort_by_key(|diagnostic| {
        diagnostic
            .primary
            .map(|span| (span.file, span.range.start, span.range.end))
    });
    tracing::debug!(diagnostics = diagnostics.len(), "lint rules evaluated");
    diagnostics
}

fn visit_statement(statement: &Statement, config: &LintConfig, output: &mut Vec<Diagnostic>) {
    match statement {
        Statement::If {
            condition,
            then_branch,
            else_if,
            else_branch,
            ..
        } => {
            visit_condition(condition, config, output);
            for statement in then_branch {
                visit_statement(statement, config, output);
            }
            for (condition, branch) in else_if {
                visit_condition(condition, config, output);
                for statement in branch {
                    visit_statement(statement, config, output);
                }
            }
            for statement in else_branch {
                visit_statement(statement, config, output);
            }
        }
        Statement::While {
            condition, body, ..
        } => {
            visit_condition(condition, config, output);
            for statement in body {
                visit_statement(statement, config, output);
            }
        }
        _ => {}
    }
}

fn visit_condition(expression: &ExpressionFact, config: &LintConfig, output: &mut Vec<Diagnostic>) {
    if expression.ty == Type::Error {
        return;
    }
    match &expression.kind {
        ExpressionKind::Binary {
            operator,
            left,
            right,
        } => {
            if operator == "!="
                && ((none_literal(left) && matches!(right.ty, Type::Script(_)))
                    || (none_literal(right) && matches!(left.ty, Type::Script(_))))
                && let Some(severity) = config.prefer_truthy_none_check
            {
                output.push(
                    Diagnostic::new(
                        PREFER_TRUTHY_NONE_CHECK,
                        severity,
                        "prefer the script reference directly as the condition",
                    )
                    .at(expression.span),
                );
            }
            visit_condition(left, config, output);
            visit_condition(right, config, output);
        }
        ExpressionKind::Unary { operand, .. }
        | ExpressionKind::Parenthesized(operand)
        | ExpressionKind::Cast { value: operand, .. } => {
            visit_condition(operand, config, output);
        }
        _ => {}
    }
}

fn none_literal(expression: &ExpressionFact) -> bool {
    expression.ty == Type::None
        && matches!(&expression.kind, ExpressionKind::Literal(text) if text.eq_ignore_ascii_case("none"))
}
