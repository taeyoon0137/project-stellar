//! Compose 출력 Kotlin 문법 검사기와 import 표 검사 (XXII.4.1·XXII.4.2 compile fixture 대체, ADR-0121 결정 6).
//!
//! DEV-MAC에는 Kotlin 컴파일러가 없습니다. 그래서 생성기가 쓰는 Kotlin 문법 부분집합을 토큰 단위로 끝까지 읽는
//! 재귀 하강 parser로 구문을 검사합니다. 읽지 못하는 토큰이 하나라도 있으면 실패합니다(모르는 구문을 건너뛰지 않음).
//!
//! - 파일: `@file:` annotation, `package`, `import`, top-level 선언.
//! - 선언: annotation, 가시성 modifier, `object`·`class`(주 생성자 매개변수), `fun`(receiver, 기본값 있는 매개변수,
//!   반환 타입, block 또는 `=` 식 본문), `val`·`var`(타입, `=` 또는 `by` 위임).
//! - 문: 선언, `if`·`else`, `return`, 대입(`=`·`+=`·`-=`), 식. 줄바꿈과 `;`가 문을 나눕니다.
//! - 식: 이항 연산(우선순위 등반), 단항(`!`·`-`·`+`), 호출(이름 있는 인자, trailing lambda), 멤버 접근(`.`·`?.`),
//!   `::class`, `!!`, 색인, lambda(`a, b ->` 매개변수), `this@label`, 문자열 template, 수 literal(`1f`, `0.5f`, `16`).
//! - 타입: 이름(점 경로), type 인자, nullable, 함수 타입(`(A) -> B`, receiver 함수 타입).
//!
//! import 표 검사([`check_imports`])는 import가 모두 생성기 import 표(`compose::IMPORTS`)의 정규 경로이고, 표의 경로가
//! 모두 알려진 Compose artifact package에 있는지 확인합니다.

use std::collections::BTreeSet;

use crate::android::compose::IMPORTS;

/// 토큰.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    /// 식별자 또는 keyword.
    Ident(String),
    /// 수 literal.
    Number,
    /// 문자열 literal(template 식은 따로 검사).
    Str,
    /// 기호.
    Punct(&'static str),
    /// 줄바꿈(문 구분).
    Newline,
}

const PUNCTS: [&str; 35] = [
    "?.", "?:", "!!", "::", "->", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "..", "(", ")",
    "{", "}", "[", "]", ",", ".", ":", ";", "=", "<", ">", "+", "-", "*", "/", "%", "!", "@", "?",
];

/// 문법 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SyntaxError {
    /// 토큰 위치.
    pub at: usize,
    /// 설명.
    pub message: String,
}

fn lex(source: &str) -> Result<Vec<Token>, SyntaxError> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let error = |at: usize, message: &str| SyntaxError {
        at,
        message: message.to_string(),
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            out.push(Token::Newline);
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            if i + 1 >= chars.len() {
                return Err(error(i, "닫히지 않은 주석"));
            }
            i += 2;
        } else if c == '"' {
            i += 1;
            loop {
                match chars.get(i) {
                    None | Some('\n') => return Err(error(i, "닫히지 않은 문자열")),
                    Some('\\') => match chars.get(i + 1) {
                        Some('n' | 't' | 'r' | 'b' | '"' | '\'' | '\\' | '$') => i += 2,
                        Some('u') => {
                            let hex: String = chars.iter().skip(i + 2).take(4).collect();
                            if hex.len() != 4 || !hex.chars().all(|h| h.is_ascii_hexdigit()) {
                                return Err(error(i, "잘못된 \\u escape"));
                            }
                            i += 6;
                        }
                        _ => return Err(error(i, "모르는 escape")),
                    },
                    Some('$') if chars.get(i + 1) == Some(&'{') => {
                        // template 식: 짝 중괄호까지 읽어 따로 검사합니다.
                        let start = i + 2;
                        let mut depth = 1;
                        let mut j = start;
                        while j < chars.len() && depth > 0 {
                            match chars[j] {
                                '{' => depth += 1,
                                '}' => depth -= 1,
                                _ => {}
                            }
                            j += 1;
                        }
                        if depth != 0 {
                            return Err(error(i, "닫히지 않은 문자열 template"));
                        }
                        let inner: String = chars[start..j - 1].iter().collect();
                        let tokens = lex(&inner)?;
                        let mut parser = Parser { tokens, pos: 0 };
                        parser.expression()?;
                        parser.skip_newlines();
                        if parser.pos != parser.tokens.len() {
                            return Err(error(i, "문자열 template 식이 끝나지 않았습니다"));
                        }
                        i = j;
                    }
                    Some('"') => {
                        i += 1;
                        break;
                    }
                    Some(_) => i += 1,
                }
            }
            out.push(Token::Str);
        } else if c.is_ascii_digit() {
            let start = i;
            if c == '0' && matches!(chars.get(i + 1), Some('x' | 'X')) {
                i += 2;
                while i < chars.len() && (chars[i].is_ascii_hexdigit() || chars[i] == '_') {
                    i += 1;
                }
            } else {
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                    i += 1;
                }
                // 소수점 뒤가 수일 때만 소수입니다(`1.dp`는 멤버 접근).
                if chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(char::is_ascii_digit)
                {
                    i += 1;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                if matches!(chars.get(i), Some('e' | 'E')) {
                    return Err(error(i, "생성기는 지수 표기를 쓰지 않습니다"));
                }
            }
            if matches!(chars.get(i), Some('f' | 'F' | 'L' | 'u' | 'U')) {
                i += 1;
            }
            if chars
                .get(i)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
            {
                return Err(error(start, "수 literal 뒤에 문자가 붙었습니다"));
            }
            out.push(Token::Number);
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Token::Ident(chars[start..i].iter().collect()));
        } else if c == '`' {
            let start = i + 1;
            i = start;
            while i < chars.len() && chars[i] != '`' && chars[i] != '\n' {
                i += 1;
            }
            if chars.get(i) != Some(&'`') || i == start {
                return Err(error(i, "잘못된 backtick 이름"));
            }
            out.push(Token::Ident(chars[start..i].iter().collect()));
            i += 1;
        } else {
            let rest: String = chars[i..chars.len().min(i + 2)].iter().collect();
            let Some(punct) = PUNCTS.iter().find(|p| rest.starts_with(**p)) else {
                return Err(error(i, &format!("모르는 문자 {c:?}")));
            };
            out.push(Token::Punct(punct));
            i += punct.chars().count();
        }
    }
    Ok(out)
}

const MODIFIERS: [&str; 9] = [
    "private",
    "internal",
    "public",
    "protected",
    "override",
    "open",
    "inline",
    "data",
    "final",
];

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

type Parsed = Result<(), SyntaxError>;

impl Parser {
    fn fail<T>(&self, message: &str) -> Result<T, SyntaxError> {
        Err(SyntaxError {
            at: self.pos,
            message: format!("{message} (토큰 {:?})", self.tokens.get(self.pos)),
        })
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset)
    }

    fn is_punct(&self, punct: &str) -> bool {
        matches!(self.peek(), Some(Token::Punct(p)) if *p == punct)
    }

    fn is_ident(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Token::Ident(w)) if w == word)
    }

    fn eat_punct(&mut self, punct: &str) -> bool {
        if self.is_punct(punct) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn eat_ident(&mut self, word: &str) -> bool {
        if self.is_ident(word) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, punct: &str) -> Parsed {
        if self.eat_punct(punct) {
            Ok(())
        } else {
            self.fail(&format!("{punct:?}가 필요합니다"))
        }
    }

    fn name(&mut self) -> Result<String, SyntaxError> {
        match self.peek() {
            Some(Token::Ident(word)) => {
                let word = word.clone();
                self.pos += 1;
                Ok(word)
            }
            _ => self.fail("이름이 필요합니다"),
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Some(Token::Newline)) {
            self.pos += 1;
        }
    }

    /// 다음 의미 토큰이 `punct`이면 줄바꿈을 건너뜁니다(`.` chain, `,` 뒤 등).
    fn newline_then(&self, punct: &str) -> bool {
        let mut i = self.pos;
        while matches!(self.tokens.get(i), Some(Token::Newline)) {
            i += 1;
        }
        matches!(self.tokens.get(i), Some(Token::Punct(p)) if *p == punct)
    }

    fn qualified(&mut self) -> Parsed {
        self.name()?;
        while self.is_punct(".") && matches!(self.peek_at(1), Some(Token::Ident(_))) {
            self.pos += 2;
        }
        Ok(())
    }

    fn file(&mut self) -> Parsed {
        self.skip_newlines();
        while self.is_punct("@") && matches!(self.peek_at(1), Some(Token::Ident(w)) if w == "file")
        {
            self.pos += 2;
            self.expect_punct(":")?;
            self.annotation_body()?;
            self.skip_newlines();
        }
        if self.eat_ident("package") {
            self.qualified()?;
            self.end_of_statement()?;
        }
        self.skip_newlines();
        while self.eat_ident("import") {
            self.qualified()?;
            if self.eat_ident("as") {
                self.name()?;
            }
            self.end_of_statement()?;
            self.skip_newlines();
        }
        while self.peek().is_some() {
            self.declaration()?;
            self.skip_newlines();
        }
        Ok(())
    }

    fn end_of_statement(&mut self) -> Parsed {
        match self.peek() {
            None | Some(Token::Newline) => {
                self.skip_newlines();
                Ok(())
            }
            Some(Token::Punct(";")) => {
                self.pos += 1;
                Ok(())
            }
            Some(Token::Punct("}")) => Ok(()),
            _ => self.fail("문 끝이 필요합니다"),
        }
    }

    fn annotation_body(&mut self) -> Parsed {
        self.qualified()?;
        if self.is_punct("(") {
            self.arguments()?;
        }
        Ok(())
    }

    fn annotations_and_modifiers(&mut self) -> Parsed {
        loop {
            if self.eat_punct("@") {
                self.annotation_body()?;
                self.skip_newlines();
            } else if matches!(self.peek(), Some(Token::Ident(w)) if MODIFIERS.contains(&w.as_str()))
            {
                self.pos += 1;
            } else {
                return Ok(());
            }
        }
    }

    fn declaration(&mut self) -> Parsed {
        self.annotations_and_modifiers()?;
        if self.eat_ident("object") {
            self.name()?;
            self.class_body()
        } else if self.eat_ident("class") {
            self.name()?;
            if self.is_punct("(") {
                self.parameters()?;
            }
            if self.is_punct("{") {
                self.class_body()?;
            }
            Ok(())
        } else if self.eat_ident("fun") {
            self.function()
        } else if self.is_ident("val") || self.is_ident("var") {
            self.property()
        } else {
            self.fail("선언이 필요합니다")
        }
    }

    fn class_body(&mut self) -> Parsed {
        self.expect_punct("{")?;
        self.skip_newlines();
        while !self.is_punct("}") {
            if self.peek().is_none() {
                return self.fail("닫히지 않은 class 본문");
            }
            self.declaration()?;
            self.end_of_statement()?;
            self.skip_newlines();
        }
        self.pos += 1;
        Ok(())
    }

    fn function(&mut self) -> Parsed {
        // receiver 타입(`RowScope.`)과 이름.
        self.name()?;
        while self.eat_punct(".") {
            self.name()?;
        }
        self.parameters()?;
        if self.eat_punct(":") {
            self.type_ref()?;
        }
        if self.eat_punct("=") {
            self.skip_newlines();
            self.expression()
        } else {
            self.block()
        }
    }

    fn parameters(&mut self) -> Parsed {
        self.expect_punct("(")?;
        self.skip_newlines();
        while !self.is_punct(")") {
            self.annotations_and_modifiers()?;
            if self.is_ident("val") || self.is_ident("var") {
                self.pos += 1;
            }
            self.name()?;
            self.expect_punct(":")?;
            self.type_ref()?;
            if self.eat_punct("=") {
                self.skip_newlines();
                self.expression()?;
            }
            self.skip_newlines();
            if !self.eat_punct(",") {
                break;
            }
            self.skip_newlines();
        }
        self.skip_newlines();
        self.expect_punct(")")
    }

    fn type_ref(&mut self) -> Parsed {
        if self.is_punct("(") {
            // 함수 타입 `(A, B) -> C`.
            self.pos += 1;
            while !self.is_punct(")") {
                self.type_ref()?;
                if !self.eat_punct(",") {
                    break;
                }
            }
            self.expect_punct(")")?;
            self.expect_punct("->")?;
            return self.type_ref();
        }
        self.name()?;
        while self.is_punct(".") && matches!(self.peek_at(1), Some(Token::Ident(_))) {
            self.pos += 2;
        }
        if self.eat_punct("<") {
            loop {
                if !self.eat_punct("*") {
                    self.type_ref()?;
                }
                if !self.eat_punct(",") {
                    break;
                }
            }
            self.expect_punct(">")?;
        }
        self.eat_punct("?");
        // receiver 함수 타입 `RowScope.() -> Unit`.
        if self.is_punct(".") && matches!(self.peek_at(1), Some(Token::Punct("("))) {
            self.pos += 1;
            return self.type_ref();
        }
        Ok(())
    }

    fn property(&mut self) -> Parsed {
        self.pos += 1; // val | var
        self.name()?;
        if self.eat_punct(":") {
            self.type_ref()?;
        }
        if self.eat_punct("=") || self.eat_ident("by") {
            self.skip_newlines();
            self.expression()?;
        }
        Ok(())
    }

    fn block(&mut self) -> Parsed {
        self.expect_punct("{")?;
        self.statements()?;
        self.expect_punct("}")
    }

    fn statements(&mut self) -> Parsed {
        loop {
            while matches!(self.peek(), Some(Token::Newline) | Some(Token::Punct(";"))) {
                self.pos += 1;
            }
            if self.is_punct("}") || self.peek().is_none() {
                return Ok(());
            }
            self.statement()?;
            match self.peek() {
                Some(Token::Newline) | Some(Token::Punct(";")) | Some(Token::Punct("}")) => {}
                None => {}
                _ => return self.fail("문이 끝나지 않았습니다"),
            }
        }
    }

    fn statement(&mut self) -> Parsed {
        if self.is_ident("val") || self.is_ident("var") {
            return self.property();
        }
        if self.eat_ident("return") {
            if !matches!(
                self.peek(),
                Some(Token::Newline) | Some(Token::Punct(";")) | Some(Token::Punct("}"))
            ) {
                self.expression()?;
            }
            return Ok(());
        }
        self.expression()?;
        if self.eat_punct("=") || self.eat_punct("+=") || self.eat_punct("-=") {
            self.skip_newlines();
            self.expression()?;
        }
        Ok(())
    }

    fn expression(&mut self) -> Parsed {
        self.binary(0)
    }

    fn binary_level(&self) -> Option<(usize, usize)> {
        // (우선순위, 토큰 수). 큰 수가 강하게 묶입니다.
        match self.peek()? {
            Token::Punct("||") => Some((1, 1)),
            Token::Punct("&&") => Some((2, 1)),
            Token::Punct("==") | Token::Punct("!=") => Some((3, 1)),
            Token::Punct("<") | Token::Punct(">") | Token::Punct("<=") | Token::Punct(">=") => {
                Some((4, 1))
            }
            Token::Ident(w) if w == "is" || w == "in" => Some((4, 1)),
            Token::Punct("?:") => Some((5, 1)),
            // infix 함수(`provides`, `to`, `until`, `and`, `or`).
            Token::Ident(w) if ["provides", "to", "until", "and", "or"].contains(&w.as_str()) => {
                Some((6, 1))
            }
            Token::Punct("..") => Some((7, 1)),
            Token::Punct("+") | Token::Punct("-") => Some((8, 1)),
            Token::Punct("*") | Token::Punct("/") => Some((9, 1)),
            Token::Ident(w) if w == "as" => Some((10, 1)),
            _ => None,
        }
    }

    fn binary(&mut self, min: usize) -> Parsed {
        self.unary()?;
        while let Some((level, width)) = self.binary_level() {
            if level < min {
                break;
            }
            let is_cast = self.is_ident("as");
            self.pos += width;
            self.skip_newlines();
            if is_cast {
                self.type_ref()?;
            } else {
                self.binary(level + 1)?;
            }
        }
        Ok(())
    }

    fn unary(&mut self) -> Parsed {
        if self.eat_punct("!") || self.eat_punct("-") || self.eat_punct("+") {
            return self.unary();
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Parsed {
        self.primary()?;
        loop {
            if self.is_punct("(") {
                self.arguments()?;
                if self.is_punct("{") {
                    self.lambda()?;
                }
            } else if self.is_punct("{") {
                // trailing lambda만 있는 호출(`remember { … }`).
                self.lambda()?;
            } else if self.newline_then(".") || self.newline_then("?.") {
                self.skip_newlines();
                self.pos += 1;
                self.name()?;
            } else if self.eat_punct("::") {
                self.name()?;
            } else if self.eat_punct("!!") {
            } else if self.eat_punct("[") {
                self.expression()?;
                self.expect_punct("]")?;
            } else {
                return Ok(());
            }
        }
    }

    fn arguments(&mut self) -> Parsed {
        self.expect_punct("(")?;
        self.skip_newlines();
        while !self.is_punct(")") {
            // 이름 있는 인자 `name = 식`.
            if matches!(self.peek(), Some(Token::Ident(_)))
                && matches!(self.peek_at(1), Some(Token::Punct("=")))
            {
                self.pos += 2;
                self.skip_newlines();
            }
            self.expression()?;
            self.skip_newlines();
            if !self.eat_punct(",") {
                break;
            }
            self.skip_newlines();
        }
        self.skip_newlines();
        self.expect_punct(")")
    }

    fn lambda(&mut self) -> Parsed {
        self.expect_punct("{")?;
        // 매개변수 `a, b ->`.
        let save = self.pos;
        self.skip_newlines();
        let mut params = true;
        loop {
            if !matches!(self.peek(), Some(Token::Ident(_))) {
                params = false;
                break;
            }
            self.pos += 1;
            if self.eat_punct("->") {
                break;
            }
            if !self.eat_punct(",") {
                params = false;
                break;
            }
            self.skip_newlines();
        }
        if !params {
            self.pos = save;
        }
        self.statements()?;
        self.expect_punct("}")
    }

    fn primary(&mut self) -> Parsed {
        match self.peek().cloned() {
            Some(Token::Number) | Some(Token::Str) => {
                self.pos += 1;
                Ok(())
            }
            Some(Token::Punct("(")) => {
                self.pos += 1;
                self.skip_newlines();
                self.expression()?;
                self.skip_newlines();
                self.expect_punct(")")
            }
            Some(Token::Punct("{")) => self.lambda(),
            Some(Token::Punct("::")) => {
                self.pos += 1;
                self.name().map(|_| ())
            }
            Some(Token::Ident(word)) => match word.as_str() {
                "if" => {
                    self.pos += 1;
                    self.expect_punct("(")?;
                    self.expression()?;
                    self.expect_punct(")")?;
                    self.skip_newlines();
                    self.branch()?;
                    if self.newline_else() {
                        self.skip_newlines();
                        self.pos += 1;
                        self.skip_newlines();
                        self.branch()?;
                    }
                    Ok(())
                }
                "this" => {
                    self.pos += 1;
                    if self.eat_punct("@") {
                        self.name()?;
                    }
                    Ok(())
                }
                "true" | "false" | "null" => {
                    self.pos += 1;
                    Ok(())
                }
                "fun" | "class" | "object" | "package" | "import" | "return" | "val" | "var"
                | "else" | "when" | "for" | "while" => self.fail("식 자리에 keyword가 왔습니다"),
                _ => {
                    self.pos += 1;
                    Ok(())
                }
            },
            _ => self.fail("식이 필요합니다"),
        }
    }

    fn newline_else(&self) -> bool {
        let mut i = self.pos;
        while matches!(self.tokens.get(i), Some(Token::Newline)) {
            i += 1;
        }
        matches!(self.tokens.get(i), Some(Token::Ident(w)) if w == "else")
    }

    fn branch(&mut self) -> Parsed {
        if self.is_punct("{") {
            self.block()
        } else {
            self.statement()
        }
    }
}

/// Kotlin 소스 하나의 구문을 검사합니다.
pub(crate) fn check_syntax(source: &str) -> Result<(), SyntaxError> {
    let tokens = lex(source)?;
    let mut parser = Parser { tokens, pos: 0 };
    parser.file()?;
    if parser.pos != parser.tokens.len() {
        return parser.fail("파일 끝이 필요합니다");
    }
    Ok(())
}

/// import한 경로가 Compose artifact에 있는 package인지 확인할 때 쓰는 package → artifact 표.
///
/// 생성기 import 표의 경로는 모두 이 package 중 하나에 있어야 합니다. package 이름은 Jetpack Compose 1.7 공개 API
/// package입니다(artifact `androidx.compose.<group>:<artifact>`).
pub(crate) const PACKAGE_ARTIFACTS: &[(&str, &str)] = &[
    ("androidx.compose.foundation", "foundation:foundation"),
    (
        "androidx.compose.foundation.interaction",
        "foundation:foundation",
    ),
    (
        "androidx.compose.foundation.layout",
        "foundation:foundation-layout",
    ),
    (
        "androidx.compose.foundation.selection",
        "foundation:foundation",
    ),
    ("androidx.compose.foundation.text", "foundation:foundation"),
    ("androidx.compose.runtime", "runtime:runtime"),
    ("androidx.compose.ui", "ui:ui"),
    ("androidx.compose.ui.draw", "ui:ui"),
    ("androidx.compose.ui.graphics", "ui:ui-graphics"),
    ("androidx.compose.ui.graphics.colorspace", "ui:ui-graphics"),
    ("androidx.compose.ui.graphics.drawscope", "ui:ui-graphics"),
    ("androidx.compose.ui.semantics", "ui:ui"),
    ("androidx.compose.ui.state", "ui:ui"),
    ("androidx.compose.ui.unit", "ui:ui-unit"),
];

/// import 줄이 모두 생성기 import 표의 정규 경로이고 중복·정렬 위반이 없는지 확인합니다.
pub(crate) fn check_imports(source: &str) -> Result<(), String> {
    let imports: Vec<&str> = source
        .lines()
        .filter_map(|line| line.strip_prefix("import "))
        .collect();
    let mut sorted = imports.clone();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted != imports {
        return Err("import가 정렬되지 않았거나 중복입니다".to_string());
    }
    for path in imports {
        let simple = path.rsplit('.').next().unwrap_or(path);
        match IMPORTS.iter().find(|(name, _)| *name == simple) {
            Some((_, canonical)) if *canonical == path => {}
            Some((_, canonical)) => {
                return Err(format!("{path}는 표의 경로 {canonical}와 다릅니다"));
            }
            None => return Err(format!("{path}는 import 표에 없습니다")),
        }
    }
    Ok(())
}

/// 생성기 import 표 자체 검사: 이름이 경로 끝 조각과 같고, package가 [`PACKAGE_ARTIFACTS`]에 있고, 이름이 겹치지
/// 않습니다.
pub(crate) fn check_import_table() -> Result<(), String> {
    let packages: BTreeSet<&str> = PACKAGE_ARTIFACTS.iter().map(|(p, _)| *p).collect();
    let mut names = BTreeSet::new();
    for (name, path) in IMPORTS {
        let (package, simple) = path
            .rsplit_once('.')
            .ok_or_else(|| format!("{path}에 package가 없습니다"))?;
        if simple != *name {
            return Err(format!("{name}의 경로 끝 조각이 {simple}입니다"));
        }
        if !packages.contains(package) {
            return Err(format!(
                "{path}의 package {package}는 Compose artifact 표에 없습니다"
            ));
        }
        if !names.insert(*name) {
            return Err(format!("{name}이 두 번 있습니다"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_checker_accepts_generator_constructs() {
        let source = r#"@file:OptIn(ExperimentalLayoutApi::class)

package a.b

import x.Row

object Colors {
    val accent = Color(red = 0.5f, green = 0f, blue = -0.05f, alpha = 1f)
}

class State {
    var memo by mutableStateOf("")
}

@Composable
fun View(
    state: State = remember { State() },
    onAction: (String) -> Unit = {},
) {
    Node1(state, onAction)
}

@Composable
private fun RowScope.Node1(state: State, onAction: (String) -> Unit) {
    Row(
        modifier = Modifier
            .weight(1f)
            .clickable(enabled = !(state.isSaving), role = Role.Button) { onAction("a"); state.memo = "b" }
            .drawWithContent { withTransform({ transform(Matrix(floatArrayOf(0f, 1f))) }) { this@drawWithContent.drawContent() } },
        decorationBox = { inner ->
            Box {
                if (state.memo.isEmpty()) BasicText(text = "메모 ${state.memo.length}")
                inner()
            }
        },
    )
}
"#;
        check_syntax(source).expect("문법");
    }

    #[test]
    fn syntax_checker_rejects_broken_sources() {
        for broken in [
            "fun f() { Row(a = ) }\n",
            "fun f() { Row(\n",
            "fun f() { val = 1 }\n",
            "fun f() { a b }\n",
            "fun f() { \"abc }\n",
            "fun f() { x(1 2) }\n",
            "object { }\n",
            "fun f() { 1.5e3 }\n",
            "fun f(a) {}\n",
            "fun f() { \"${a b}\" }\n",
            "import\n",
            "fun f() { if (a) }\n",
        ] {
            assert!(check_syntax(broken).is_err(), "{broken}");
        }
    }

    #[test]
    fn import_table_has_known_packages() {
        check_import_table().expect("import 표");
        assert!(check_imports("import androidx.compose.ui.Modifier\n").is_ok());
        assert!(check_imports("import androidx.compose.ui.layout.Modifier\n").is_err());
        assert!(check_imports("import androidx.compose.ui.Nope\n").is_err());
        assert!(check_imports(
            "import androidx.compose.ui.unit.dp\nimport androidx.compose.ui.Modifier\n"
        )
        .is_err());
    }
}
