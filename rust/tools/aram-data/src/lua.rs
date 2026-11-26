//! Minimal Lua table parser for the League Wiki `Module:ChampionData/data`
//! page, mirroring the Python generator's lexer/parser (strings, numbers,
//! numeric keys, unary signs and `+ - * /` arithmetic).

#[derive(Debug, Clone, PartialEq)]
pub enum LuaValue {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    Table(Vec<(String, LuaValue)>),
    Array(Vec<LuaValue>),
}

impl LuaValue {
    pub fn get(&self, key: &str) -> Option<&LuaValue> {
        match self {
            LuaValue::Table(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            LuaValue::Str(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_num(&self) -> Option<f64> {
        match self {
            LuaValue::Num(value) => Some(*value),
            _ => None,
        }
    }

    pub fn values(&self) -> Vec<&LuaValue> {
        match self {
            LuaValue::Table(entries) => entries.iter().map(|(_, value)| value).collect(),
            LuaValue::Array(items) => items.iter().collect(),
            _ => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Eq,
    Comma,
    Semicolon,
    Plus,
    Minus,
    Star,
    Slash,
    Str(String),
    Num(f64),
    Ident(String),
    Eof,
}

struct Lexer {
    chars: Vec<char>,
    index: usize,
}

impl Lexer {
    fn new(text: &str) -> Self {
        Self {
            chars: text.chars().collect(),
            index: 0,
        }
    }

    fn skip_ignored(&mut self) {
        while self.index < self.chars.len() {
            let current = self.chars[self.index];
            if current.is_whitespace() {
                self.index += 1;
            } else if self.peek_str("--[[") {
                let mut end = self.index + 4;
                while end + 1 < self.chars.len()
                    && !(self.chars[end] == ']' && self.chars[end + 1] == ']')
                {
                    end += 1;
                }
                self.index = (end + 2).min(self.chars.len());
            } else if self.peek_str("--") {
                while self.index < self.chars.len() && self.chars[self.index] != '\n' {
                    self.index += 1;
                }
            } else {
                break;
            }
        }
    }

    fn peek_str(&self, needle: &str) -> bool {
        let mut expected = needle.chars();
        let mut actual = self.chars[self.index..].iter();
        loop {
            match expected.next() {
                None => return true,
                Some(ch) => match actual.next() {
                    Some(value) if *value == ch => {}
                    _ => return false,
                },
            }
        }
    }

    fn next(&mut self) -> Result<Token, String> {
        self.skip_ignored();
        let Some(current) = self.chars.get(self.index).copied() else {
            return Ok(Token::Eof);
        };
        self.index += 1;
        let token = match current {
            '{' => Token::LBrace,
            '}' => Token::RBrace,
            '[' => Token::LBracket,
            ']' => Token::RBracket,
            '=' => Token::Eq,
            ',' => Token::Comma,
            ';' => Token::Semicolon,
            '+' => Token::Plus,
            '-' => Token::Minus,
            '*' => Token::Star,
            '/' => Token::Slash,
            '"' | '\'' => Token::Str(self.string(current)?),
            '.' if self.chars.get(self.index).is_some_and(char::is_ascii_digit) => {
                self.index -= 1;
                Token::Num(self.number()?)
            }
            ch if ch.is_ascii_digit() => {
                self.index -= 1;
                Token::Num(self.number()?)
            }
            ch if ch.is_alphabetic() || ch == '_' => Token::Ident(self.ident()),
            other => return Err(format!("unexpected character {other:?}")),
        };
        Ok(token)
    }

    fn string(&mut self, quote: char) -> Result<String, String> {
        let mut buffer = String::new();
        while let Some(current) = self.chars.get(self.index).copied() {
            self.index += 1;
            if current == '\\' {
                let Some(next) = self.chars.get(self.index).copied() else {
                    break;
                };
                self.index += 1;
                buffer.push(match next {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '\\' => '\\',
                    '"' => '"',
                    '\'' => '\'',
                    other => other,
                });
                continue;
            }
            if current == quote {
                return Ok(buffer);
            }
            buffer.push(current);
        }
        Err("unterminated string".into())
    }

    fn number(&mut self) -> Result<f64, String> {
        let start = self.index;
        while self.chars.get(self.index).is_some_and(char::is_ascii_digit) {
            self.index += 1;
        }
        if self.chars.get(self.index) == Some(&'.') {
            self.index += 1;
            while self.chars.get(self.index).is_some_and(char::is_ascii_digit) {
                self.index += 1;
            }
        }
        if matches!(self.chars.get(self.index), Some('e' | 'E')) {
            self.index += 1;
            if matches!(self.chars.get(self.index), Some('+' | '-')) {
                self.index += 1;
            }
            while self.chars.get(self.index).is_some_and(char::is_ascii_digit) {
                self.index += 1;
            }
        }
        let text = self.chars[start..self.index].iter().collect::<String>();
        text.parse::<f64>()
            .map_err(|_| format!("invalid number {text:?}"))
    }

    fn ident(&mut self) -> String {
        let start = self.index - 1;
        while self
            .chars
            .get(self.index)
            .is_some_and(|ch| ch.is_alphanumeric() || *ch == '_')
        {
            self.index += 1;
        }
        self.chars[start..self.index].iter().collect()
    }
}

struct Parser {
    lexer: Lexer,
    token: Token,
}

impl Parser {
    fn new(text: &str) -> Result<Self, String> {
        let mut lexer = Lexer::new(text);
        let token = lexer.next()?;
        Ok(Self { lexer, token })
    }

    fn advance(&mut self) -> Result<(), String> {
        self.token = self.lexer.next()?;
        Ok(())
    }

    fn parse_module(&mut self) -> Result<LuaValue, String> {
        loop {
            match &self.token {
                Token::Eof => return Err("module has no return statement".into()),
                Token::Ident(name) if name == "return" => {
                    self.advance()?;
                    return self.value();
                }
                _ => self.advance()?,
            }
        }
    }

    fn value(&mut self) -> Result<LuaValue, String> {
        match &self.token {
            Token::LBrace => self.table(),
            Token::Str(text) => {
                let value = LuaValue::Str(text.clone());
                self.advance()?;
                Ok(value)
            }
            Token::Num(_) | Token::Plus | Token::Minus => Ok(LuaValue::Num(self.expression()?)),
            Token::Ident(name) => {
                let name = name.clone();
                self.advance()?;
                match name.as_str() {
                    "true" => Ok(LuaValue::Bool(true)),
                    "false" => Ok(LuaValue::Bool(false)),
                    "nil" => Ok(LuaValue::Nil),
                    _ => Err(format!("unexpected identifier {name:?}")),
                }
            }
            other => Err(format!("unexpected token {other:?}")),
        }
    }

    fn expression(&mut self) -> Result<f64, String> {
        let mut value = self.term()?;
        loop {
            let operator = match self.token {
                Token::Plus => '+',
                Token::Minus => '-',
                _ => return Ok(value),
            };
            self.advance()?;
            let operand = self.term()?;
            value = if operator == '+' {
                value + operand
            } else {
                value - operand
            };
        }
    }

    fn term(&mut self) -> Result<f64, String> {
        let mut value = self.factor()?;
        loop {
            let operator = match self.token {
                Token::Star => '*',
                Token::Slash => '/',
                _ => return Ok(value),
            };
            self.advance()?;
            let operand = self.factor()?;
            value = if operator == '*' {
                value * operand
            } else {
                value / operand
            };
        }
    }

    fn factor(&mut self) -> Result<f64, String> {
        match &self.token {
            Token::Plus | Token::Minus => {
                let negative = self.token == Token::Minus;
                self.advance()?;
                let value = self.factor()?;
                Ok(if negative { -value } else { value })
            }
            Token::Num(value) => {
                let value = *value;
                self.advance()?;
                Ok(value)
            }
            other => Err(format!("expected number, got {other:?}")),
        }
    }

    fn table(&mut self) -> Result<LuaValue, String> {
        self.advance()?;
        let mut keyed = Vec::new();
        let mut array = Vec::new();
        while self.token != Token::RBrace {
            match &self.token {
                Token::Eof => return Err("unterminated table".into()),
                Token::LBracket => {
                    self.advance()?;
                    let key = match &self.token {
                        Token::Str(text) => {
                            let key = text.clone();
                            self.advance()?;
                            key
                        }
                        Token::Num(_) | Token::Plus | Token::Minus => {
                            let value = self.expression()?;
                            if value.fract() == 0.0 {
                                format!("{}", value as i64)
                            } else {
                                format!("{value}")
                            }
                        }
                        other => return Err(format!("expected table key, got {other:?}")),
                    };
                    if self.token != Token::RBracket {
                        return Err(format!("expected ']', got {:?}", self.token));
                    }
                    self.advance()?;
                    if self.token != Token::Eq {
                        return Err(format!("expected '=', got {:?}", self.token));
                    }
                    self.advance()?;
                    let value = self.value()?;
                    keyed.push((key, value));
                }
                Token::Ident(name) => {
                    let key = name.clone();
                    self.advance()?;
                    if self.token != Token::Eq {
                        return Err(format!("expected '=', got {:?}", self.token));
                    }
                    self.advance()?;
                    let value = self.value()?;
                    keyed.push((key, value));
                }
                _ => array.push(self.value()?),
            }
            match self.token {
                Token::Comma | Token::Semicolon => self.advance()?,
                Token::RBrace => {}
                _ => return Err(format!("expected ',' or '}}', got {:?}", self.token)),
            }
        }
        self.advance()?;
        if keyed.is_empty() {
            Ok(LuaValue::Array(array))
        } else {
            for (index, item) in array.into_iter().enumerate() {
                keyed.push(((index + 1).to_string(), item));
            }
            Ok(LuaValue::Table(keyed))
        }
    }
}

pub fn parse_module(text: &str) -> Result<LuaValue, String> {
    Parser::new(text)?.parse_module()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numeric_keys_and_arithmetic() {
        let text = "return { [\"X\"] = { id = 1, title = \"X\", \
                    stats = { aram = { dmg_dealt = 0.95 } }, \
                    skills = {[1] = \"A\", [2] = \"B\"}, hp = 84+1000/17, delta = -0.05 } }";
        let module = parse_module(text).unwrap();
        let x = module.get("X").unwrap();
        let skills = x.get("skills").unwrap();
        assert_eq!(skills.get("2").unwrap().as_str(), Some("B"));
        assert!((x.get("hp").unwrap().as_num().unwrap() - (84.0 + 1000.0 / 17.0)).abs() < 1e-9);
        assert!((x.get("delta").unwrap().as_num().unwrap() + 0.05).abs() < 1e-9);
    }

    #[test]
    fn skips_comments_and_handles_escapes() {
        let text = "-- line\n--[[ block ]] return { [\"a\"] = \"x\\\"y\" }";
        let module = parse_module(text).unwrap();
        assert_eq!(module.get("a").unwrap().as_str(), Some("x\"y"));
    }

    #[test]
    fn parse_numbers_with_exponents() {
        let text = "return { n = 1.5e2 }";
        let value = parse_module(text).unwrap();
        assert_eq!(value.get("n").unwrap().as_num(), Some(150.0));
    }
}
