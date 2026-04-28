use super::ast::*;
use super::lexer::Token;
use crate::error::{Result, XPathError};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self { Self { tokens, pos: 0 } }

    fn peek(&self) -> &Token { &self.tokens[self.pos] }
    fn peek_n(&self, n: usize) -> &Token {
        self.tokens.get(self.pos + n).unwrap_or(&Token::Eof)
    }
    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        self.pos += 1; t
    }
    fn eat(&mut self, t: &Token) -> bool {
        if self.peek() == t { self.pos += 1; true } else { false }
    }
    fn expect(&mut self, t: &Token) -> Result<()> {
        if self.eat(t) { Ok(()) }
        else { Err(XPathError::ParseError(format!("expected {:?}, got {:?}", t, self.peek()))) }
    }

    pub fn parse_expr(&mut self) -> Result<Expr> {
        let e = self.parse_or()?;
        if !matches!(self.peek(), Token::Eof) {
            return Err(XPathError::ParseError(format!("trailing token {:?}", self.peek())));
        }
        Ok(e)
    }

    fn parse_or(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_and()?;
        while matches!(self.peek(), Token::Or) {
            self.bump();
            let rhs = self.parse_and()?;
            lhs = Expr::BinaryOp(BinOp::Or, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_equality()?;
        while matches!(self.peek(), Token::And) {
            self.bump();
            let rhs = self.parse_equality()?;
            lhs = Expr::BinaryOp(BinOp::And, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_equality(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_relational()?;
        loop {
            let op = match self.peek() {
                Token::Eq => BinOp::Eq,
                Token::NotEq => BinOp::NotEq,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_relational()?;
            lhs = Expr::BinaryOp(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_relational(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_additive()?;
        loop {
            let op = match self.peek() {
                Token::Lt => BinOp::Lt, Token::Le => BinOp::Le,
                Token::Gt => BinOp::Gt, Token::Ge => BinOp::Ge,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_additive()?;
            lhs = Expr::BinaryOp(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_additive(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_multiplicative()?;
            lhs = Expr::BinaryOp(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Token::Star => BinOp::Mul,
                Token::Div => BinOp::Div,
                Token::Mod => BinOp::Mod,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_unary()?;
            lhs = Expr::BinaryOp(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        if matches!(self.peek(), Token::Minus) {
            self.bump();
            let e = self.parse_unary()?;
            Ok(Expr::UnaryMinus(Box::new(e)))
        } else {
            self.parse_union()
        }
    }

    fn parse_union(&mut self) -> Result<Expr> {
        let first = self.parse_path_expr()?;
        if !matches!(self.peek(), Token::Pipe) { return Ok(first); }
        let mut parts = vec![first];
        while self.eat(&Token::Pipe) {
            parts.push(self.parse_path_expr()?);
        }
        Ok(Expr::Union(parts))
    }

    /// PathExpr := LocationPath | FilterExpr (('/' | '//') RelativeLocationPath)?
    fn parse_path_expr(&mut self) -> Result<Expr> {
        // 判断是否以 primary 开头
        if self.starts_primary() {
            let primary = self.parse_filter_expr_primary()?;
            // FilterExpr 可能跟 / //
            let predicates = Vec::new();
            // primary 后的 predicate 已在 parse_filter_expr_primary 中处理
            let steps = if matches!(self.peek(), Token::Slash | Token::DoubleSlash) {
                self.parse_path_continuation()?
            } else { Vec::new() };

            if steps.is_empty() && predicates.is_empty() {
                Ok(primary)
            } else {
                // 把 primary 抽取的 predicates 合并 - 此处未使用
                Ok(match primary {
                    Expr::Filter { primary, predicates: ps, .. } =>
                        Expr::Filter { primary, predicates: ps, steps },
                    other => Expr::Filter { primary: Box::new(other), predicates, steps },
                })
            }
        } else {
            // LocationPath
            let path = self.parse_location_path()?;
            Ok(Expr::Path(path))
        }
    }

    fn starts_primary(&self) -> bool {
        match self.peek() {
            Token::VarRef(_) | Token::LParen | Token::String(_) | Token::Number(_) => true,
            Token::Name(n) => {
                // FunctionCall: Name '('，但要排除 NodeType: node/text/comment/processing-instruction
                if matches!(self.peek_n(1), Token::LParen) {
                    !matches!(n.as_str(), "node" | "text" | "comment" | "processing-instruction")
                } else { false }
            }
            _ => false,
        }
    }

    fn parse_filter_expr_primary(&mut self) -> Result<Expr> {
        let primary = self.parse_primary_expr()?;
        let mut preds = Vec::new();
        while matches!(self.peek(), Token::LBracket) {
            self.bump();
            let p = self.parse_or()?;
            self.expect(&Token::RBracket)?;
            preds.push(p);
        }
        if preds.is_empty() { Ok(primary) }
        else {
            Ok(Expr::Filter { primary: Box::new(primary), predicates: preds, steps: Vec::new() })
        }
    }

    fn parse_primary_expr(&mut self) -> Result<Expr> {
        match self.bump() {
            Token::VarRef(n) => Ok(Expr::VarRef(n)),
            Token::LParen => {
                let e = self.parse_or()?;
                self.expect(&Token::RParen)?;
                Ok(e)
            }
            Token::String(s) => Ok(Expr::String(s)),
            Token::Number(n) => Ok(Expr::Number(n)),
            Token::Name(name) => {
                // FunctionCall
                self.expect(&Token::LParen)?;
                let mut args = Vec::new();
                if !matches!(self.peek(), Token::RParen) {
                    args.push(self.parse_or()?);
                    while self.eat(&Token::Comma) {
                        args.push(self.parse_or()?);
                    }
                }
                self.expect(&Token::RParen)?;
                Ok(Expr::FunctionCall { name, args })
            }
            t => Err(XPathError::ParseError(format!("primary expr expected, got {:?}", t))),
        }
    }

    /// 在 FilterExpr 之后继续 / // step (/ step)*
    fn parse_path_continuation(&mut self) -> Result<Vec<Step>> {
        let mut steps = Vec::new();
        while matches!(self.peek(), Token::Slash | Token::DoubleSlash) {
            let dbl = matches!(self.peek(), Token::DoubleSlash);
            self.bump();
            if dbl {
                steps.push(Step {
                    axis: Axis::DescendantOrSelf,
                    test: NodeTest::Node,
                    predicates: Vec::new(),
                });
            }
            steps.push(self.parse_step()?);
        }
        Ok(steps)
    }

    fn parse_location_path(&mut self) -> Result<PathExpr> {
        let mut absolute = false;
        let mut leading_desc = false;
        let mut steps = Vec::new();

        match self.peek() {
            Token::Slash => { absolute = true; self.bump(); }
            Token::DoubleSlash => {
                absolute = true; leading_desc = true; self.bump();
                steps.push(Step {
                    axis: Axis::DescendantOrSelf,
                    test: NodeTest::Node,
                    predicates: Vec::new(),
                });
            }
            _ => {}
        }

        // 绝对路径但没有后续 step 是合法的 (/)
        let must_have_step = !absolute || leading_desc || self.starts_step();
        if must_have_step && self.starts_step() {
            steps.push(self.parse_step()?);
            while matches!(self.peek(), Token::Slash | Token::DoubleSlash) {
                let dbl = matches!(self.peek(), Token::DoubleSlash);
                self.bump();
                if dbl {
                    steps.push(Step {
                        axis: Axis::DescendantOrSelf,
                        test: NodeTest::Node,
                        predicates: Vec::new(),
                    });
                }
                steps.push(self.parse_step()?);
            }
        }
        Ok(PathExpr { absolute, leading_descendant: leading_desc, steps })
    }

    fn starts_step(&self) -> bool {
        matches!(self.peek(),
            Token::At | Token::Dot | Token::DoubleDot | Token::Star | Token::Name(_))
    }

    fn parse_step(&mut self) -> Result<Step> {
        // Abbreviated steps
        if matches!(self.peek(), Token::Dot) {
            self.bump();
            return Ok(Step { axis: Axis::Self_, test: NodeTest::Node, predicates: Vec::new() });
        }
        if matches!(self.peek(), Token::DoubleDot) {
            self.bump();
            return Ok(Step { axis: Axis::Parent, test: NodeTest::Node, predicates: Vec::new() });
        }

        let axis = if matches!(self.peek(), Token::At) {
            self.bump(); Axis::Attribute
        } else if let Token::Name(n) = self.peek().clone() {
            if matches!(self.peek_n(1), Token::DoubleColon) {
                let ax = parse_axis_name(&n)?;
                self.bump(); self.bump();
                ax
            } else { Axis::Child }
        } else { Axis::Child };

        let test = self.parse_node_test()?;
        let mut preds = Vec::new();
        while matches!(self.peek(), Token::LBracket) {
            self.bump();
            preds.push(self.parse_or()?);
            self.expect(&Token::RBracket)?;
        }
        Ok(Step { axis, test, predicates: preds })
    }

    fn parse_node_test(&mut self) -> Result<NodeTest> {
        match self.peek().clone() {
            Token::Star => { self.bump(); Ok(NodeTest::Wildcard) }
            Token::Name(name) => {
                // NodeType?
                if matches!(self.peek_n(1), Token::LParen) {
                    match name.as_str() {
                        "node" => { self.bump(); self.bump(); self.expect(&Token::RParen)?; return Ok(NodeTest::Node); }
                        "text" => { self.bump(); self.bump(); self.expect(&Token::RParen)?; return Ok(NodeTest::Text); }
                        "comment" => { self.bump(); self.bump(); self.expect(&Token::RParen)?; return Ok(NodeTest::Comment); }
                        "processing-instruction" => {
                            self.bump(); self.bump();
                            let arg = if let Token::String(s) = self.peek().clone() {
                                self.bump(); Some(s)
                            } else { None };
                            self.expect(&Token::RParen)?;
                            return Ok(NodeTest::ProcessingInstruction(arg));
                        }
                        _ => {}
                    }
                }
                self.bump();
                // 处理 prefix:local
                if self.eat(&Token::Colon) {
                    if let Token::Name(local) = self.bump() {
                        return Ok(NodeTest::Name(format!("{}:{}", name, local)));
                    } else if self.eat(&Token::Star) {
                        return Ok(NodeTest::Wildcard);
                    } else {
                        return Err(XPathError::ParseError("expected name after ':'".into()));
                    }
                }
                Ok(NodeTest::Name(name))
            }
            t => Err(XPathError::ParseError(format!("expected node test, got {:?}", t))),
        }
    }
}

fn parse_axis_name(s: &str) -> Result<Axis> {
    Ok(match s {
        "child" => Axis::Child,
        "descendant" => Axis::Descendant,
        "descendant-or-self" => Axis::DescendantOrSelf,
        "parent" => Axis::Parent,
        "ancestor" => Axis::Ancestor,
        "ancestor-or-self" => Axis::AncestorOrSelf,
        "self" => Axis::Self_,
        "attribute" => Axis::Attribute,
        "namespace" => Axis::Namespace,
        "following" => Axis::Following,
        "preceding" => Axis::Preceding,
        "following-sibling" => Axis::FollowingSibling,
        "preceding-sibling" => Axis::PrecedingSibling,
        _ => return Err(XPathError::ParseError(format!("unknown axis '{}'", s))),
    })
}
