use rowan::Language;

use super::{Parser, SyntaxErrorKind, SyntaxKind};

impl Parser<'_> {
    pub(super) fn expression(&mut self, min_binding_power: u8) {
        self.trivia();
        let checkpoint = self.builder.checkpoint();
        if !self.expression_prefix() {
            return;
        }
        loop {
            if self.kind() == Some(SyntaxKind::Dot) {
                self.builder.start_node_at(
                    checkpoint,
                    crate::PapyrusLanguage::kind_to_raw(SyntaxKind::MemberExpr),
                );
                self.bump_kind(SyntaxKind::Dot);
                self.expect_kind(SyntaxKind::Ident, "expected member name");
                self.finish();
                continue;
            }
            if self.kind() == Some(SyntaxKind::LBracket) {
                self.builder.start_node_at(
                    checkpoint,
                    crate::PapyrusLanguage::kind_to_raw(SyntaxKind::IndexExpr),
                );
                self.bump_kind(SyntaxKind::LBracket);
                self.expression(0);
                self.expect_kind(SyntaxKind::RBracket, "expected ]");
                self.finish();
                continue;
            }
            if self.kind() == Some(SyntaxKind::LParen) {
                self.builder.start_node_at(
                    checkpoint,
                    crate::PapyrusLanguage::kind_to_raw(SyntaxKind::CallExpr),
                );
                self.bump_kind(SyntaxKind::LParen);
                while !self.done() && self.kind() != Some(SyntaxKind::RParen) && !self.at_line_end()
                {
                    let before = self.cursor;
                    let argument = self.builder.checkpoint();
                    self.expression(0);
                    if self.kind() == Some(SyntaxKind::Equals) {
                        self.builder.start_node_at(
                            argument,
                            crate::PapyrusLanguage::kind_to_raw(SyntaxKind::NamedArgument),
                        );
                        self.bump_kind(SyntaxKind::Equals);
                        self.expression(0);
                        self.finish();
                    }
                    if self.cursor == before || !self.bump_kind(SyntaxKind::Comma) {
                        break;
                    }
                }
                self.expect_kind(SyntaxKind::RParen, "expected )");
                self.finish();
                continue;
            }
            let (left, right) = match self.kind() {
                Some(SyntaxKind::Star | SyntaxKind::Slash | SyntaxKind::Percent) => (11, 12),
                Some(SyntaxKind::Plus | SyntaxKind::Minus) => (9, 10),
                Some(
                    SyntaxKind::Less
                    | SyntaxKind::Greater
                    | SyntaxKind::LessEq
                    | SyntaxKind::GreaterEq
                    | SyntaxKind::EqEq
                    | SyntaxKind::NotEq,
                ) => (7, 8),
                Some(SyntaxKind::Ident) if self.at_keyword("as") => (15, 16),
                Some(SyntaxKind::AndAnd) => (3, 4),
                Some(SyntaxKind::OrOr) => (1, 2),
                _ => break,
            };
            if left < min_binding_power {
                break;
            }
            self.builder.start_node_at(
                checkpoint,
                crate::PapyrusLanguage::kind_to_raw(SyntaxKind::BinaryExpr),
            );
            self.trivia();
            let cast = self.at_keyword("as");
            self.bump();
            if cast {
                self.type_ref();
            } else {
                self.expression(right);
            }
            self.finish();
        }
    }

    /// Parse the expression operand before postfix and binary binding.
    fn expression_prefix(&mut self) -> bool {
        match self.kind() {
            Some(SyntaxKind::Plus | SyntaxKind::Minus | SyntaxKind::Bang) => {
                self.start(SyntaxKind::UnaryExpr);
                self.bump();
                self.expression(13);
                self.finish();
            },
            Some(SyntaxKind::Ident) if self.at_keyword("new") => {
                self.start(SyntaxKind::NewArrayExpr);
                self.bump_keyword("new");
                self.type_name("expected array element type");
                self.expect_kind(SyntaxKind::LBracket, "expected [");
                self.expression(0);
                self.expect_kind(SyntaxKind::RBracket, "expected ]");
                self.finish();
            },
            Some(SyntaxKind::Ident) => {
                let literal = self.text().is_some_and(|text| {
                    text.eq_ignore_ascii_case("true")
                        || text.eq_ignore_ascii_case("false")
                        || text.eq_ignore_ascii_case("none")
                });
                self.start(if literal {
                    SyntaxKind::LiteralExpr
                } else {
                    SyntaxKind::NameExpr
                });
                self.bump();
                self.finish();
            },
            Some(SyntaxKind::Number | SyntaxKind::String | SyntaxKind::UnclosedString) => {
                self.start(SyntaxKind::LiteralExpr);
                self.bump();
                self.finish();
            },
            Some(SyntaxKind::LParen) => {
                self.start(SyntaxKind::ParenExpr);
                self.bump();
                self.expression(0);
                self.expect_kind(SyntaxKind::RParen, "expected )");
                self.finish();
            },
            _ => {
                self.issue_missing(SyntaxErrorKind::ExpectedExpression, "expected expression");
                return false;
            },
        }
        true
    }
}
