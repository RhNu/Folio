//! Statement boundaries and control-flow bodies retain the shared lossless parser state.
use rowan::Language;

use super::{Parser, SyntaxErrorKind, SyntaxKind};

impl Parser<'_> {
    pub(super) fn statement(&mut self) {
        self.start(SyntaxKind::Statement);
        if self.at_keyword("if") {
            self.if_statement();
            self.finish();
            return;
        }
        if self.at_keyword("while") {
            self.while_statement();
            self.finish();
            return;
        }
        if self.is_unsupported_statement() {
            self.error_line(
                SyntaxErrorKind::UnsupportedStatement,
                "unsupported statement",
            );
            self.finish();
            return;
        }
        if self.at_keyword("return") {
            self.start(SyntaxKind::ReturnStmt);
            self.bump_keyword("return");
            if !self.at_line_end() {
                self.expression(0);
            }
            self.finish();
        } else if self.starts_variable() {
            self.variable();
            self.finish();
            return;
        } else if !self.at_line_end() {
            let checkpoint = self.builder.checkpoint();
            self.expression(0);
            if matches!(
                self.kind(),
                Some(
                    SyntaxKind::Equals
                        | SyntaxKind::PlusEq
                        | SyntaxKind::MinusEq
                        | SyntaxKind::StarEq
                        | SyntaxKind::SlashEq
                        | SyntaxKind::PercentEq
                )
            ) {
                self.builder.start_node_at(
                    checkpoint,
                    crate::PapyrusLanguage::kind_to_raw(SyntaxKind::AssignmentStmt),
                );
                self.trivia();
                self.bump();
                self.expression(0);
                self.finish();
            }
        }
        self.line_tail();
        self.finish();
    }

    fn if_statement(&mut self) {
        self.start(SyntaxKind::IfStmt);
        self.bump_keyword("if");
        self.expression(0);
        self.line_tail();
        self.control_block(&["elseif", "else", "endif"]);
        while self.at_keyword("elseif") {
            self.start(SyntaxKind::ElseIfClause);
            self.bump_keyword("elseif");
            self.expression(0);
            self.line_tail();
            self.control_block(&["elseif", "else", "endif"]);
            self.finish();
        }
        if self.at_keyword("else") {
            self.start(SyntaxKind::ElseClause);
            self.bump_keyword("else");
            self.line_tail();
            self.control_block(&["endif"]);
            self.finish();
        }
        if self.bump_keyword("endif") {
            self.line_tail();
        } else {
            self.issue_missing(SyntaxErrorKind::MissingEndIf, "expected EndIf");
        }
        self.finish();
    }

    fn while_statement(&mut self) {
        self.start(SyntaxKind::WhileStmt);
        self.bump_keyword("while");
        self.expression(0);
        self.line_tail();
        self.control_block(&["endwhile"]);
        if self.bump_keyword("endwhile") {
            self.line_tail();
        } else {
            self.issue_missing(SyntaxErrorKind::MissingEndWhile, "expected EndWhile");
        }
        self.finish();
    }

    /// Leave enclosing terminators and new declarations for their owning parser frame.
    fn control_block(&mut self, terminators: &[&str]) {
        self.start(SyntaxKind::Block);
        loop {
            self.eat_blank();
            if self.done()
                || terminators.iter().any(|word| self.at_keyword(word))
                || [
                    "endfunction",
                    "endevent",
                    "endstate",
                    "endproperty",
                    "else",
                    "elseif",
                    "endif",
                    "endwhile",
                ]
                .iter()
                .any(|word| self.at_keyword(word))
                || self.at_keyword("scriptname")
                || self.starts_function()
                || self.starts_state()
                || self.starts_property()
                || self.at_keyword("event")
            {
                break;
            }
            self.statement();
        }
        self.finish();
    }
}
