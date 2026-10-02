use crate::{LexToken, PapyrusDialect, Parse, SyntaxError, SyntaxErrorKind, SyntaxKind, lex};
use folio_source::TextRange;
use rowan::{GreenNodeBuilder, Language};

/// Parses the supported Papyrus subset and retains all source text on errors.
pub fn parse(source: &str, dialect: PapyrusDialect) -> Parse {
    let _ = dialect;
    let mut parser = Parser {
        source,
        tokens: lex(source),
        cursor: 0,
        builder: GreenNodeBuilder::new(),
        errors: Vec::new(),
    };
    for token in &parser.tokens {
        let message = match token.kind {
            SyntaxKind::UnclosedString => Some("unclosed string"),
            SyntaxKind::UnclosedComment => Some("unclosed comment"),
            _ => None,
        };
        if let Some(message) = message {
            parser.errors.push(SyntaxError {
                kind: if token.kind == SyntaxKind::UnclosedString {
                    SyntaxErrorKind::UnclosedString
                } else {
                    SyntaxErrorKind::UnclosedComment
                },
                range: token.range,
                message: message.to_owned(),
            });
        }
    }
    parser
        .builder
        .start_node(crate::PapyrusLanguage::kind_to_raw(SyntaxKind::Root));
    while !parser.done() {
        parser.eat_blank();
        if parser.done() {
            break;
        }
        if parser.at_keyword("scriptname") {
            parser.script();
        } else if parser.at_keyword("import") {
            parser.import();
        } else if parser.starts_state() {
            parser.state();
        } else if parser.at_keyword("event") {
            parser.event();
        } else if parser.starts_property() {
            parser.property();
        } else if parser.starts_function() {
            parser.function();
        } else if parser.at_keyword("return") {
            parser.error_line(
                SyntaxErrorKind::ReturnOutsideFunction,
                "return outside function",
            );
        } else if parser.starts_variable() {
            parser.variable();
        } else {
            parser.error_line(
                SyntaxErrorKind::ExpectedDeclaration,
                "expected script or function declaration",
            );
        }
    }
    parser.builder.finish_node();
    let green = parser.builder.finish();
    debug_assert_eq!(green.text_len(), rowan::TextSize::from(source.len() as u32));
    crate::source_structure::validate(
        &crate::SyntaxNode::new_root(green.clone()),
        source,
        &mut parser.errors,
    );
    tracing::debug!(
        bytes = source.len(),
        errors = parser.errors.len(),
        "parsed Papyrus source"
    );
    Parse {
        green,
        errors: parser.errors,
    }
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<LexToken>,
    cursor: usize,
    builder: GreenNodeBuilder<'static>,
    errors: Vec<SyntaxError>,
}

impl Parser<'_> {
    fn done(&self) -> bool {
        self.cursor >= self.tokens.len()
    }

    fn significant(&self) -> Option<usize> {
        (self.cursor..self.tokens.len()).find(|&index| !self.inline_trivia(index))
    }

    fn inline_trivia(&self, index: usize) -> bool {
        let token = self.tokens[index];
        let has_newline =
            token.text(self.source).contains('\r') || token.text(self.source).contains('\n');
        (token.kind.is_trivia() && !has_newline) || token.kind == SyntaxKind::Continuation
    }

    fn newline_like(&self, index: usize) -> bool {
        let token = self.tokens[index];
        token.kind == SyntaxKind::Newline
            || matches!(
                token.kind,
                SyntaxKind::Comment | SyntaxKind::UnclosedComment
            ) && (token.kind == SyntaxKind::UnclosedComment
                || token.text(self.source).contains('\r')
                || token.text(self.source).contains('\n'))
    }

    fn kind(&self) -> Option<SyntaxKind> {
        self.significant().map(|index| self.tokens[index].kind)
    }

    fn text(&self) -> Option<&str> {
        self.significant()
            .map(|index| self.tokens[index].text(self.source))
    }

    fn at_keyword(&self, keyword: &str) -> bool {
        self.kind() == Some(SyntaxKind::Ident)
            && self
                .text()
                .is_some_and(|text| text.eq_ignore_ascii_case(keyword))
    }

    fn starts_function(&self) -> bool {
        let words = self.line_words();
        matches!(words.as_slice(), [(SyntaxKind::Ident, first), ..] if first.eq_ignore_ascii_case("function"))
            || matches!(words.as_slice(), [(SyntaxKind::Ident, _), (SyntaxKind::Ident, second), ..] if second.eq_ignore_ascii_case("function"))
            || matches!(words.as_slice(), [(SyntaxKind::Ident, _), (SyntaxKind::LBracket, _), (SyntaxKind::RBracket, _), (SyntaxKind::Ident, fourth), ..] if fourth.eq_ignore_ascii_case("function"))
    }

    fn starts_variable(&self) -> bool {
        if self.is_unsupported_statement() {
            return false;
        }
        let words = self
            .line_words()
            .into_iter()
            .map(|(kind, _)| kind)
            .collect::<Vec<_>>();
        matches!(words.as_slice(), [SyntaxKind::Ident, SyntaxKind::Ident, ..])
            || matches!(
                words.as_slice(),
                [
                    SyntaxKind::Ident,
                    SyntaxKind::LBracket,
                    SyntaxKind::RBracket,
                    SyntaxKind::Ident,
                    ..
                ]
            )
    }

    fn line_words(&self) -> Vec<(SyntaxKind, &str)> {
        self.tokens[self.cursor..]
            .iter()
            .take_while(|token| !self.newline_like_token(**token))
            .filter(|token| !token.kind.is_trivia())
            .map(|token| (token.kind, token.text(self.source)))
            .collect()
    }

    fn newline_like_token(&self, token: LexToken) -> bool {
        token.kind == SyntaxKind::Newline
            || matches!(
                token.kind,
                SyntaxKind::Comment | SyntaxKind::UnclosedComment
            ) && token.text(self.source).contains(['\r', '\n'])
    }

    fn starts_state(&self) -> bool {
        self.at_keyword("state")
            || matches!(self.line_words().as_slice(), [(SyntaxKind::Ident, first), (SyntaxKind::Ident, second), ..]
                if first.eq_ignore_ascii_case("auto") && second.eq_ignore_ascii_case("state"))
    }

    fn starts_property(&self) -> bool {
        let words = self.line_words();
        matches!(words.as_slice(), [(SyntaxKind::Ident, _), (SyntaxKind::Ident, second), (SyntaxKind::Ident, _), ..]
            if second.eq_ignore_ascii_case("property"))
            || matches!(words.as_slice(), [(SyntaxKind::Ident, _), (SyntaxKind::LBracket, _), (SyntaxKind::RBracket, _), (SyntaxKind::Ident, fourth), ..]
                if fourth.eq_ignore_ascii_case("property"))
    }

    fn is_unsupported_statement(&self) -> bool {
        self.text().is_some_and(|word| {
            [
                "if",
                "elseif",
                "else",
                "endif",
                "while",
                "endwhile",
                "event",
                "endevent",
                "state",
                "endstate",
                "property",
                "endproperty",
                "auto",
                "autoreadonly",
                "import",
                "group",
                "endgroup",
            ]
            .iter()
            .any(|keyword| word.eq_ignore_ascii_case(keyword))
        })
    }

    fn start(&mut self, kind: SyntaxKind) {
        self.builder
            .start_node(crate::PapyrusLanguage::kind_to_raw(kind));
    }
    fn finish(&mut self) {
        self.builder.finish_node();
    }

    fn bump(&mut self) {
        let token = self.tokens[self.cursor];
        self.builder.token(
            crate::PapyrusLanguage::kind_to_raw(token.kind),
            token.text(self.source),
        );
        self.cursor += 1;
    }

    fn trivia(&mut self) {
        while !self.done() && self.inline_trivia(self.cursor) {
            self.bump();
        }
    }

    fn eat_blank(&mut self) {
        loop {
            self.trivia();
            if self
                .significant()
                .is_some_and(|index| self.newline_like(index))
            {
                self.trivia();
                self.bump();
            } else {
                break;
            }
        }
    }

    fn bump_kind(&mut self, kind: SyntaxKind) -> bool {
        if self.kind() == Some(kind) {
            self.trivia();
            self.bump();
            true
        } else {
            false
        }
    }

    fn bump_keyword(&mut self, keyword: &str) -> bool {
        if self.at_keyword(keyword) {
            self.trivia();
            self.bump();
            true
        } else {
            false
        }
    }

    fn issue(&mut self, kind: SyntaxErrorKind, message: &str) {
        let range = self
            .significant()
            .map(|index| self.tokens[index].range)
            .unwrap_or(TextRange {
                start: self.source.len(),
                end: self.source.len(),
            });
        self.errors.push(SyntaxError {
            kind,
            range,
            message: message.to_owned(),
        });
    }

    fn issue_missing(&mut self, kind: SyntaxErrorKind, message: &str) {
        let offset = self
            .significant()
            .map(|index| self.tokens[index].range.start)
            .unwrap_or(self.source.len());
        self.errors.push(SyntaxError {
            kind,
            range: TextRange {
                start: offset,
                end: offset,
            },
            message: message.to_owned(),
        });
        self.builder
            .token(crate::PapyrusLanguage::kind_to_raw(SyntaxKind::Missing), "");
    }

    fn expect_kind(&mut self, kind: SyntaxKind, message: &str) {
        if !self.bump_kind(kind) {
            self.issue_missing(SyntaxErrorKind::Missing(kind), message);
        }
    }

    fn name(&mut self, message: &str) {
        if self.kind() == Some(SyntaxKind::Ident) && self.text().is_some_and(reserved_word) {
            self.issue(
                SyntaxErrorKind::UnexpectedText,
                "reserved word cannot be a declaration name",
            );
        }
        self.expect_kind(SyntaxKind::Ident, message);
    }

    fn type_name(&mut self, message: &str) {
        if self.kind() == Some(SyntaxKind::Ident)
            && self.text().is_some_and(|text| {
                reserved_word(text)
                    && !["int", "float", "string", "bool"]
                        .iter()
                        .any(|ty| text.eq_ignore_ascii_case(ty))
            })
        {
            self.issue(
                SyntaxErrorKind::UnexpectedText,
                "reserved word cannot be a type name",
            );
        }
        self.expect_kind(SyntaxKind::Ident, message);
    }

    fn script(&mut self) {
        self.start(SyntaxKind::ScriptDecl);
        self.bump_keyword("scriptname");
        self.name("expected script name");
        if self.bump_keyword("extends") {
            self.name("expected parent script name");
        }
        self.flags();
        self.line_tail();
        self.finish();
    }

    fn import(&mut self) {
        self.start(SyntaxKind::ImportDecl);
        self.bump_keyword("import");
        self.name("expected imported script name");
        self.line_tail();
        self.finish();
    }

    /// Declaration flags are parsed as words and validated against the project's flag set later.
    fn flags(&mut self) -> bool {
        let mut native = false;
        while !self.at_line_end() && self.kind() == Some(SyntaxKind::Ident) {
            native |= self.at_keyword("native");
            self.trivia();
            self.bump();
        }
        native
    }

    fn state(&mut self) {
        self.start(SyntaxKind::StateDecl);
        self.bump_keyword("auto");
        self.bump_keyword("state");
        self.name("expected state name");
        self.flags();
        self.line_tail();
        self.start(SyntaxKind::Block);
        loop {
            self.eat_blank();
            if self.done() || self.at_keyword("scriptname") || self.starts_state() {
                self.issue_missing(SyntaxErrorKind::MissingEndState, "expected EndState");
                break;
            }
            if self.bump_keyword("endstate") {
                self.line_tail();
                break;
            }
            if self.at_keyword("event") {
                self.event();
            } else if self.starts_function() {
                self.function();
            } else {
                self.error_line(
                    SyntaxErrorKind::ExpectedDeclaration,
                    "expected state member",
                );
            }
        }
        self.finish();
        self.finish();
    }

    fn property(&mut self) {
        self.start(SyntaxKind::PropertyDecl);
        self.type_ref();
        self.bump_keyword("property");
        self.name("expected property name");
        if self.bump_kind(SyntaxKind::Equals) {
            self.expression(0);
        }
        let automatic = self.at_keyword("auto") || self.at_keyword("autoreadonly");
        self.flags();
        self.line_tail();
        if !automatic {
            self.start(SyntaxKind::Block);
            loop {
                self.eat_blank();
                if self.done()
                    || self.at_keyword("scriptname")
                    || self.starts_state()
                    || self.starts_property()
                    || self.at_keyword("event")
                {
                    self.issue_missing(SyntaxErrorKind::MissingEndProperty, "expected EndProperty");
                    break;
                }
                if self.bump_keyword("endproperty") {
                    self.line_tail();
                    break;
                }
                if self.starts_function() {
                    self.function();
                } else {
                    self.error_line(
                        SyntaxErrorKind::ExpectedDeclaration,
                        "expected property accessor",
                    );
                }
            }
            self.finish();
        }
        self.finish();
    }

    fn event(&mut self) {
        self.start(SyntaxKind::EventDecl);
        self.bump_keyword("event");
        self.name("expected event name");
        self.parameters();
        let native = self.flags();
        self.line_tail();
        if !native {
            self.body("endevent", SyntaxErrorKind::MissingEndEvent);
        }
        self.finish();
    }

    fn function(&mut self) {
        self.start(SyntaxKind::FunctionDecl);
        if !self.at_keyword("function") {
            self.type_ref();
        }
        if !self.bump_keyword("function") {
            self.issue_missing(
                SyntaxErrorKind::MissingFunctionKeyword,
                "expected Function keyword",
            );
        }
        self.name("expected function name");
        self.parameters();
        let native = self.flags();
        self.line_tail();
        if native {
            self.finish();
            return;
        }
        self.body("endfunction", SyntaxErrorKind::MissingEndFunction);
        self.finish();
    }

    fn body(&mut self, closing: &str, missing: SyntaxErrorKind) {
        self.start(SyntaxKind::Block);
        loop {
            self.eat_blank();
            if self.done() {
                self.issue_missing(missing, &format!("expected {closing}"));
                break;
            }
            if self.at_keyword(closing) {
                self.bump_keyword(closing);
                self.line_tail();
                break;
            }
            // A declaration at line start is a recovery boundary for a missing EndFunction.
            if self.at_keyword("scriptname")
                || self.starts_function()
                || self.at_keyword("event")
                || self.starts_property()
                || self.starts_state()
                || self.at_keyword("endstate")
                || self.at_keyword("endproperty")
                || (closing != "endfunction" && self.at_keyword("endfunction"))
                || (closing != "endevent" && self.at_keyword("endevent"))
            {
                self.issue_missing(
                    missing,
                    &format!("expected {closing} before next declaration"),
                );
                break;
            }
            self.statement();
        }
        self.finish();
    }

    fn type_ref(&mut self) {
        self.start(SyntaxKind::TypeRef);
        self.type_name("expected type");
        if self.bump_kind(SyntaxKind::LBracket) {
            self.expect_kind(SyntaxKind::RBracket, "expected ]");
        }
        self.finish();
    }

    fn variable(&mut self) {
        self.start(SyntaxKind::VariableDecl);
        self.type_ref();
        self.name("expected variable name");
        if self.bump_kind(SyntaxKind::Equals) {
            self.expression(0);
        }
        self.flags();
        self.line_tail();
        self.finish();
    }

    fn parameters(&mut self) {
        self.start(SyntaxKind::ParameterList);
        if !self.bump_kind(SyntaxKind::LParen) {
            self.issue_missing(SyntaxErrorKind::Missing(SyntaxKind::LParen), "expected (");
            self.finish();
            return;
        }
        while !self.done()
            && self.kind() != Some(SyntaxKind::RParen)
            && self.kind() != Some(SyntaxKind::Newline)
        {
            let before = self.cursor;
            self.start(SyntaxKind::Parameter);
            self.type_ref();
            self.name("expected parameter name");
            if self.bump_kind(SyntaxKind::Equals) {
                self.expression(0);
            }
            self.finish();
            if self.cursor == before {
                break;
            }
            if !self.bump_kind(SyntaxKind::Comma) {
                break;
            }
        }
        self.expect_kind(SyntaxKind::RParen, "expected )");
        self.finish();
    }

    fn statement(&mut self) {
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

    fn at_line_end(&self) -> bool {
        self.significant()
            .is_none_or(|index| self.newline_like(index))
    }

    fn line_tail(&mut self) {
        self.trivia();
        if !self.at_line_end() {
            self.issue(SyntaxErrorKind::UnexpectedText, "unexpected text on line");
            self.start(SyntaxKind::Error);
            while !self.done() && !self.at_line_end() {
                self.bump();
            }
            self.finish();
        }
        if self.at_line_end() && !self.done() {
            self.bump();
        }
    }

    fn error_line(&mut self, kind: SyntaxErrorKind, message: &str) {
        self.issue(kind, message);
        self.start(SyntaxKind::Error);
        while !self.done() && !self.at_line_end() {
            self.bump();
        }
        if !self.done() {
            self.bump();
        }
        self.finish();
    }

    fn expression(&mut self, min_binding_power: u8) {
        self.trivia();
        let checkpoint = self.builder.checkpoint();
        match self.kind() {
            Some(SyntaxKind::Plus | SyntaxKind::Minus | SyntaxKind::Bang) => {
                self.start(SyntaxKind::UnaryExpr);
                self.bump();
                self.expression(13);
                self.finish();
            }
            Some(SyntaxKind::Ident) if self.at_keyword("new") => {
                self.start(SyntaxKind::NewArrayExpr);
                self.bump_keyword("new");
                self.type_name("expected array element type");
                self.expect_kind(SyntaxKind::LBracket, "expected [");
                self.expression(0);
                self.expect_kind(SyntaxKind::RBracket, "expected ]");
                self.finish();
            }
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
            }
            Some(SyntaxKind::Number | SyntaxKind::String | SyntaxKind::UnclosedString) => {
                self.start(SyntaxKind::LiteralExpr);
                self.bump();
                self.finish();
            }
            Some(SyntaxKind::LParen) => {
                self.start(SyntaxKind::ParenExpr);
                self.bump();
                self.expression(0);
                self.expect_kind(SyntaxKind::RParen, "expected )");
                self.finish();
            }
            _ => {
                self.issue_missing(SyntaxErrorKind::ExpectedExpression, "expected expression");
                return;
            }
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
}

fn reserved_word(text: &str) -> bool {
    folio_profiles::is_skyrim_keyword(text)
}

#[cfg(test)]
mod tests;
