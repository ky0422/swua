#![allow(unused_imports)]

use crate::{
    codegen::{
        types::{AstArrayTypeKind, AstType, AstTypeKind},
        ArrayLiteral, AssignExpression, BinaryExpression, Block, BooleanLiteral, CallExpression,
        CastExpression, Declaration, DereferenceExpression, Expression,
        ExternalFunctionDeclaration, FloatLiteral, FunctionDefinition, Identifier, IfStatement,
        IndexExpression, IntLiteral, LetStatement, Literal, Parameter, PointerExpression,
        ReturnStatement, SizeofExpression, Statement, StringLiteral, StructDeclaration,
        StructLiteral, TypeDeclaration, TypeofExpression, UnaryExpression, While,
    },
    lexer::{
        tokens::{Token, TokenKind},
        Lexer, LexingError, LexingErrorKind,
    },
    BinaryOperator, DisplayNode, Position, Priority, Program, Span, UnaryOperator,
};
use std::{collections::BTreeMap, fmt};

#[derive(Debug, Clone, PartialEq)]
pub struct ParsingError {
    pub kind: ParsingErrorKind,
    pub span: Span,
}

macro_rules! impl_error_kind {
    (
        $(
            $ident:ident$(($($arg:ident: $ty:ty),*))?:
            $fn:ident$(<$gen:ident$(: $gen_1:tt$(+$gen_n:tt)*)?>)? $(($($param_ty:ty),*))? => $fmt:expr
        ),*
    ) => {
        #[derive(Debug, Clone, PartialEq)]
        pub enum ParsingErrorKind {
            LexingError(LexingErrorKind),
            $($ident$(($($ty),*))?,)*
        }

        impl fmt::Display for ParsingErrorKind {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    Self::LexingError(kind) => write!(f, "{kind} (Lexing Error)"),
                    $(Self::$ident$(($($arg),*))? => write!(f, $fmt),)*
                }
            }
        }

        impl ParsingError {
            pub fn new(kind: ParsingErrorKind, span: Span) -> Self {
                Self { kind, span }
            }

            $(
                pub fn $fn<$($gen$(: $gen_1$(+$gen_n)*)?)?>($($($arg: $param_ty,)*)? span: Span) -> Self
                {
                    Self::new(ParsingErrorKind::$ident$(($($arg.to_string()),*))?, span)
                }
            )*
        }

        impl From<LexingError> for ParsingError {
            fn from(error: LexingError) -> Self {
                Self::new(ParsingErrorKind::LexingError(error.kind), error.span)
            }
        }
    };
}

impl_error_kind! {
    ExpectedNextToken(expected: String, got: String): expected_next_token<T: ToString>(T, T) => "expected `{expected}` but got `{got}`",
    ExpectedType(expected: String): expected_ty<T: ToString>(T) => "expected type `{expected}`",
    ExpectedExpression(expected: String): expected_expression<T: ToString>(T) => "expected expression `{expected}`",
    UnexpectedToken(token: String): unexpected_token<T: ToString>(T) => "unexpected token `{token}`"
}

pub type ParseResult<T> = Result<T, ParsingError>;

macro_rules! identifier {
    ($self:ident) => {
        match $self.current_token.kind {
            $crate::lexer::tokens::TokenKind::Identifier(ref ident) => Identifier {
                identifier: ident.to_string(),
                span: $self.span,
            },
            _ => {
                return Err(ParsingError::expected_next_token(
                    "Identifier".to_string(),
                    $self.current_token.kind.to_string(),
                    $self.span,
                ))
            }
        }
    };
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parser<T>
where
    T: Iterator<Item = Token> + Default,
{
    tokens: T,
    current_token: Token,
    peek_token: Token,
    span: Span,
}

impl<T> Parser<T>
where
    T: Iterator<Item = Token> + Default,
{
    pub fn new(tokens: T) -> Self {
        let mut parser = Self {
            tokens,
            ..Default::default()
        };

        parser.next_token();
        parser.next_token();

        parser
    }

    pub fn parse_program(&mut self) -> ParseResult<Program> {
        let mut statements = Vec::new();

        while self.current_token.kind != TokenKind::EOF {
            if self.current_token.kind == TokenKind::Newline {
                self.next_token();
                continue;
            }

            statements.push(self.parse_statement()?);
        }

        Ok(Program { statements })
    }

    fn next_token(&mut self) {
        self.current_token = self.peek_token.clone();
        self.span = self.current_token.span;

        self.peek_token = self.tokens.next().unwrap_or(Token {
            kind: TokenKind::EOF,
            span: self.span,
        });
    }

    fn expect_token(&mut self, expected: TokenKind) -> ParseResult<()> {
        if self.current_token.kind == expected {
            self.next_token();
            Ok(())
        } else {
            Err(ParsingError::expected_next_token(
                expected.to_string(),
                self.current_token.kind.to_string(),
                self.current_token.span,
            ))
        }
    }

    fn is_terminated(&self, kind: TokenKind) -> bool {
        matches!(
            kind,
            TokenKind::Semicolon | TokenKind::Newline | TokenKind::EOF
        )
    }

    fn expect_termination(&mut self) -> ParseResult<()> {
        if !self.is_terminated(self.current_token.kind.clone()) {
            return Err(ParsingError::unexpected_token(
                self.current_token.kind.to_string(),
                self.span,
            ));
        }

        if self.current_token.kind == TokenKind::Semicolon {
            self.next_token();
        }

        Ok(())
    }

    fn current_priority(&self) -> Priority {
        Priority::from(self.current_token.kind.clone())
    }

    fn peek_priority(&self) -> Priority {
        Priority::from(self.peek_token.kind.clone())
    }

    fn parse_statement(&mut self) -> ParseResult<Statement> {
        Ok(match self.current_token.kind {
            TokenKind::Let => Statement::Let(self.parse_let_statement()?),
            TokenKind::Define => Statement::Function(self.parse_function_definition()?),
            TokenKind::Extern => {
                Statement::ExternalFunction(self.parse_external_function_declaration()?)
            }
            TokenKind::Return => Statement::Return(self.parse_return_statement()?),
            TokenKind::If => Statement::If(self.parse_if_statement()?),
            TokenKind::Type => Statement::Type(self.parse_type_statement()?),
            TokenKind::Declare => Statement::Declaration(self.parse_declare_statement()?),
            TokenKind::Struct => Statement::Struct(self.parse_struct_declaration()?),
            TokenKind::While => Statement::While(self.parse_while_statement()?),
            _ => self.parse_expression_statement()?,
        })
    }

    fn parse_let_statement(&mut self) -> ParseResult<LetStatement> {
        self.next_token();

        let identifier = identifier! { self };
        self.next_token();

        let ty = if self.current_token.kind == TokenKind::Colon {
            self.next_token();

            Some(self.parse_ty()?)
        } else {
            None
        };

        self.expect_token(TokenKind::Assign)?;

        let value = self.parse_expression(Priority::Lowest)?;
        self.next_token();

        if self.current_token.kind == TokenKind::Semicolon {
            self.next_token();
        }

        Ok(LetStatement {
            name: identifier,
            ty,
            value,
            span: self.span,
        })
    }

    fn parse_function_definition(&mut self) -> ParseResult<FunctionDefinition> {
        let position = self.span.start;
        self.next_token();

        let identifier = identifier! { self };
        self.next_token();

        let mut parameters = Vec::new();

        if self.current_token.kind == TokenKind::LParen {
            self.next_token();

            if self.current_token.kind != TokenKind::RParen {
                let identifier = identifier! { self };
                self.next_token();

                let ty = self.parse_ty()?;

                parameters.push(Parameter {
                    name: identifier,
                    ty,
                });

                if self.current_token.kind == TokenKind::Comma {
                    self.next_token();
                }

                while self.current_token.kind != TokenKind::RParen {
                    let identifier = identifier! { self };
                    self.next_token();

                    let ty = self.parse_ty()?;

                    parameters.push(Parameter {
                        name: identifier,
                        ty,
                    });

                    if self.current_token.kind == TokenKind::RParen {
                        break;
                    }

                    self.expect_token(TokenKind::Comma)?;
                }

                if self.current_token.kind != TokenKind::RParen {
                    return Err(ParsingError::expected_next_token(
                        TokenKind::RParen.to_string(),
                        self.current_token.kind.to_string(),
                        self.span,
                    ));
                }
            }

            self.next_token();
        }

        self.expect_token(TokenKind::Arrow)?;

        let return_type = self.parse_ty()?;

        self.expect_token(TokenKind::Assign)?;

        let body = self.parse_block(true)?;
        self.next_token();

        Ok(FunctionDefinition {
            name: identifier,
            parameters,
            return_type,
            body,
            span: Span::new(position, self.span.end),
        })
    }

    fn parse_block(&mut self, check_indent: bool) -> ParseResult<Block> {
        let position = self.span.start;
        if check_indent {
            self.next_token();
            self.expect_token(TokenKind::Indent)?;
        }

        let mut statements = Vec::new();

        while self.current_token.kind != TokenKind::Dedent
            && self.current_token.kind != TokenKind::EOF
        {
            if self.current_token.kind == TokenKind::Newline {
                self.next_token();
                continue;
            }

            statements.push(self.parse_statement()?);
        }

        if self.current_token.kind != TokenKind::Dedent && self.current_token.kind != TokenKind::EOF
        {
            return Err(ParsingError::expected_next_token(
                TokenKind::Dedent.to_string(),
                self.current_token.kind.to_string(),
                self.span,
            ));
        }

        Ok(Block {
            statements,
            span: Span::new(position, self.span.end),
        })
    }

    fn parse_external_function_declaration(&mut self) -> ParseResult<ExternalFunctionDeclaration> {
        self.next_token();

        let identifier = identifier! { self };
        self.next_token();

        let mut parameters = Vec::new();

        if self.current_token.kind == TokenKind::LParen {
            self.next_token();

            if self.current_token.kind != TokenKind::RParen {
                let ty = self.parse_ty()?;
                parameters.push(ty);

                if self.current_token.kind == TokenKind::Comma {
                    self.next_token();
                }

                while self.current_token.kind != TokenKind::RParen {
                    let ty = self.parse_ty()?;
                    parameters.push(ty);

                    if self.current_token.kind == TokenKind::RParen {
                        break;
                    }

                    self.expect_token(TokenKind::Comma)?;
                }

                if self.current_token.kind != TokenKind::RParen {
                    return Err(ParsingError::expected_next_token(
                        TokenKind::RParen.to_string(),
                        self.current_token.kind.to_string(),
                        self.span,
                    ));
                }
            }

            self.next_token();
        }

        self.expect_token(TokenKind::Arrow)?;

        let return_type = self.parse_ty()?;

        self.expect_termination()?;

        Ok(ExternalFunctionDeclaration {
            name: identifier,
            parameters,
            return_type,
            span: self.span,
        })
    }

    fn parse_return_statement(&mut self) -> ParseResult<ReturnStatement> {
        self.next_token();

        let expression = self.parse_expression(Priority::Lowest)?;
        self.next_token();

        self.expect_termination()?;

        Ok(ReturnStatement {
            value: expression,
            span: self.span,
        })
    }

    fn parse_if_statement(&mut self) -> ParseResult<IfStatement> {
        let position = self.span.start;
        self.next_token();

        let condition = self.parse_expression(Priority::Lowest)?;
        self.next_token();

        let consequence = self.parse_block(true)?;
        self.next_token();

        let alternative = if self.current_token.kind == TokenKind::Else {
            self.next_token();

            if self.current_token.kind == TokenKind::If {
                let position = self.span.start;
                Some(Block {
                    statements: vec![Statement::If(self.parse_if_statement()?)],
                    span: Span::new(position, self.span.end),
                })
            } else {
                let block = self.parse_block(true)?;
                self.next_token();

                Some(block)
            }
        } else {
            None
        };

        Ok(IfStatement {
            condition: Box::new(condition),
            consequence,
            alternative,
            span: Span::new(position, self.span.end),
        })
    }

    fn parse_type_statement(&mut self) -> ParseResult<TypeDeclaration> {
        todo!()
    }

    fn parse_declare_statement(&mut self) -> ParseResult<Declaration> {
        todo!()
    }

    fn parse_struct_declaration(&mut self) -> ParseResult<StructDeclaration> {
        todo!()
    }

    fn parse_while_statement(&mut self) -> ParseResult<While> {
        let position = self.span.start;
        self.next_token();

        let condition = self.parse_expression(Priority::Lowest)?;
        self.next_token();

        let body = self.parse_block(true)?;
        self.next_token();

        Ok(While {
            condition,
            body,
            span: Span::new(position, self.span.end),
        })
    }

    fn parse_expression_statement(&mut self) -> ParseResult<Statement> {
        let expression = self.parse_expression(Priority::Lowest)?;
        self.next_token();

        self.expect_termination()?;

        Ok(Statement::Expression(expression))
    }

    fn parse_expression(&mut self, priority: Priority) -> ParseResult<Expression> {
        let left_expression = match self.current_token.kind.clone() {
            TokenKind::Identifier(value) => {
                Some(Ok(Expression::Literal(Literal::Identifier(Identifier {
                    identifier: value.to_string(),
                    span: self.span,
                }))))
            }
            TokenKind::Int(value) => Some(Ok(Expression::Literal(Literal::Int(IntLiteral {
                value,
                span: self.span,
            })))),
            TokenKind::Float(value) => {
                Some(Ok(Expression::Literal(Literal::Float(FloatLiteral {
                    value,
                    span: self.span,
                }))))
            }
            TokenKind::String(value) => {
                Some(Ok(Expression::Literal(Literal::String(StringLiteral {
                    value: value.to_string(),
                    span: self.span,
                }))))
            }
            TokenKind::Boolean(value) => {
                Some(Ok(Expression::Literal(Literal::Boolean(BooleanLiteral {
                    value,
                    span: self.span,
                }))))
            }
            TokenKind::Bang | TokenKind::Minus => {
                let operator: UnaryOperator = self.current_token.kind.clone().into();
                self.next_token();

                Some(Ok(Expression::Unary(UnaryExpression {
                    operator,
                    expression: Box::new(self.parse_expression(Priority::Prefix)?),
                    span: self.span,
                })))
            }
            TokenKind::LParen => {
                self.next_token();

                let expression = self.parse_expression(Priority::Lowest);
                self.next_token();

                if self.current_token.kind != TokenKind::RParen {
                    return Err(ParsingError::expected_next_token(
                        TokenKind::RParen.to_string(),
                        self.current_token.kind.to_string(),
                        self.span,
                    ));
                }

                Some(expression)
            }
            // TokenKind::LBrace => Some(Ok(Expression::Block(self.parse_block_expression()?))),
            TokenKind::LBracket => Some(Ok(Expression::Literal(Literal::Array(
                self.parse_array_literal()?,
            )))),
            TokenKind::Typeof => {
                self.next_token();

                Some(Ok(Expression::Typeof(TypeofExpression {
                    expression: Box::new(self.parse_expression(Priority::Lowest)?),
                    span: self.span,
                })))
            }
            TokenKind::Sizeof => {
                self.next_token();

                Some(Ok(Expression::Sizeof(SizeofExpression {
                    expression: Box::new(self.parse_expression(Priority::Lowest)?),
                    span: self.span,
                })))
            }
            TokenKind::Asterisk => {
                self.next_token();

                Some(Ok(Expression::Dereference(DereferenceExpression {
                    expression: Box::new(self.parse_expression(Priority::Prefix)?),
                    span: self.span,
                })))
            }
            TokenKind::Ampersand => {
                self.next_token();

                Some(Ok(Expression::Pointer(PointerExpression {
                    expression: Box::new(self.parse_expression(Priority::Prefix)?),
                    span: self.span,
                })))
            }
            _ => None,
        };

        if left_expression.is_none() && !self.is_terminated(self.current_token.kind.clone()) {
            return Err(ParsingError::unexpected_token(
                self.current_token.kind.to_string(),
                self.span,
            ));
        }

        let mut left_expression = left_expression.ok_or_else(|| {
            ParsingError::unexpected_token(self.current_token.kind.to_string(), self.span)
        })?;

        while !self.is_terminated(self.peek_token.kind.clone()) && priority < self.peek_priority() {
            self.next_token();

            left_expression = match self.current_token.kind {
                TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Dot
                | TokenKind::Slash
                | TokenKind::Asterisk
                | TokenKind::Percent
                | TokenKind::EQ
                | TokenKind::NEQ
                | TokenKind::LT
                | TokenKind::GT
                | TokenKind::LTE
                | TokenKind::GTE => {
                    let operator = self.current_token.kind.clone().into();

                    let priority = self.current_priority();
                    self.next_token();
                    let right = Box::new(self.parse_expression(priority)?);

                    Ok(Expression::Binary(BinaryExpression {
                        left: Box::new(left_expression?),
                        operator,
                        right,
                        span: self.span,
                    }))
                }
                TokenKind::Assign => {
                    self.next_token();

                    let value = self.parse_expression(Priority::Lowest)?;
                    Ok(Expression::Assign(AssignExpression {
                        expression: Box::new(left_expression?),
                        value: Box::new(value),
                        span: self.span,
                    }))
                }
                TokenKind::LParen => {
                    self.next_token();

                    let mut arguments = Vec::new();

                    if self.current_token.kind != TokenKind::RParen {
                        arguments.push(self.parse_expression(Priority::Lowest)?);
                        self.next_token();

                        if self.current_token.kind == TokenKind::Comma {
                            self.next_token();
                        }

                        while self.current_token.kind != TokenKind::RParen {
                            arguments.push(self.parse_expression(Priority::Lowest)?);
                            self.next_token();

                            if self.current_token.kind == TokenKind::RParen {
                                break;
                            }

                            self.expect_token(TokenKind::Comma)?;
                        }

                        if self.current_token.kind != TokenKind::RParen {
                            return Err(ParsingError::expected_next_token(
                                TokenKind::RParen.to_string(),
                                self.current_token.kind.to_string(),
                                self.span,
                            ));
                        }
                    }

                    Ok(Expression::Call(CallExpression {
                        function: Box::new(left_expression?),
                        arguments,
                        span: self.span,
                    }))
                }
                TokenKind::LBracket => {
                    self.next_token();

                    let index = self.parse_expression(Priority::Lowest)?;
                    self.next_token();

                    if self.current_token.kind != TokenKind::RBracket {
                        return Err(ParsingError::expected_next_token(
                            TokenKind::RBracket.to_string(),
                            self.current_token.kind.to_string(),
                            self.span,
                        ));
                    }

                    Ok(Expression::Index(IndexExpression {
                        left: Box::new(left_expression?),
                        index: Box::new(index),
                        span: self.span,
                    }))
                }
                TokenKind::LBrace => {
                    let identifier = match left_expression? {
                        Expression::Literal(Literal::Identifier(identifier)) => identifier,
                        _ => {
                            return Err(ParsingError::expected_next_token(
                                "Identifier".to_string(),
                                self.current_token.kind.to_string(),
                                self.span,
                            ))
                        }
                    };

                    self.expect_token(TokenKind::LBrace)?;

                    let mut fields = BTreeMap::new();

                    while self.current_token.kind != TokenKind::RBrace {
                        let key = identifier! { self };
                        self.next_token();

                        self.expect_token(TokenKind::Colon)?;

                        fields.insert(
                            key.identifier.clone(),
                            self.parse_expression(Priority::Lowest)?,
                        );
                        self.next_token();

                        if self.current_token.kind == TokenKind::RBrace {
                            break;
                        }

                        self.expect_token(TokenKind::Comma)?;
                    }

                    if self.current_token.kind != TokenKind::RBrace {
                        return Err(ParsingError::expected_next_token(
                            TokenKind::RBrace.to_string(),
                            self.current_token.kind.to_string(),
                            self.span,
                        ));
                    }

                    Ok(Expression::Literal(Literal::Struct(StructLiteral {
                        name: identifier,
                        fields,
                        span: self.span,
                    })))
                }
                TokenKind::As => {
                    self.next_token();

                    Ok(Expression::Cast(CastExpression {
                        expression: Box::new(left_expression?),
                        cast_ty: self.parse_ty()?,
                        span: self.span,
                    }))
                }
                _ => Err(ParsingError::unexpected_token(
                    self.current_token.kind.to_string(),
                    self.span,
                )),
            };
        }

        left_expression
    }

    fn parse_array_literal(&mut self) -> ParseResult<ArrayLiteral> {
        let position = self.span.start;
        self.next_token();

        let mut elements = Vec::new();

        if self.current_token.kind == TokenKind::RBracket {
            return Ok(ArrayLiteral {
                elements,
                span: Span::new(position, self.span.end),
            });
        }

        while self.current_token.kind != TokenKind::RBrace {
            elements.push(self.parse_expression(Priority::Lowest)?);
            self.next_token();

            if self.current_token.kind == TokenKind::RBracket {
                break;
            }

            self.expect_token(TokenKind::Comma)?;
        }

        if self.current_token.kind != TokenKind::RBracket {
            return Err(ParsingError::expected_next_token(
                TokenKind::RBracket.to_string(),
                self.current_token.kind.to_string(),
                self.span,
            ));
        }

        Ok(ArrayLiteral {
            elements,
            span: Span::new(position, self.span.end),
        })
    }

    fn parse_ty(&mut self) -> ParseResult<AstType> {
        let mut ty = match &self.current_token.kind {
            TokenKind::IntType => Ok(AstTypeKind::Int),
            TokenKind::FloatType => Ok(AstTypeKind::Float),
            TokenKind::StringType => Ok(AstTypeKind::String),
            TokenKind::BooleanType => Ok(AstTypeKind::Boolean),
            TokenKind::VoidType => Ok(AstTypeKind::Void),
            TokenKind::Define => todo!(),
            TokenKind::At => {
                self.next_token();

                Ok(AstTypeKind::TypeAlias(identifier! { self }))
            }
            TokenKind::Identifier(identifier) => Ok(AstTypeKind::Struct(Identifier {
                identifier: identifier.to_string(),
                span: self.span,
            })),
            _ => Err(ParsingError::unexpected_token(
                self.current_token.kind.to_string(),
                self.span,
            )),
        };

        if self.peek_token.kind == TokenKind::LBracket {
            self.next_token();
            self.next_token();

            let mut size = None;

            if self.current_token.kind != TokenKind::RBracket {
                let expression = self.parse_expression(Priority::Lowest)?;
                self.next_token();

                if self.current_token.kind != TokenKind::RBracket {
                    return Err(ParsingError::expected_next_token(
                        TokenKind::RBracket.to_string(),
                        self.current_token.kind.to_string(),
                        self.span,
                    ));
                }

                match expression {
                    Expression::Literal(Literal::Int(int)) => {
                        size = Some(int.value as usize);
                    }
                    _ => {
                        return Err(ParsingError::expected_next_token(
                            "Int".to_string(),
                            self.current_token.kind.to_string(),
                            self.span,
                        ));
                    }
                }
            }

            ty = Ok(AstTypeKind::Array(AstArrayTypeKind {
                ty: Box::new(AstType {
                    kind: ty?,
                    span: self.span,
                }),
                len: size,
                span: self.span,
            }));
        }

        if self.peek_token.kind == TokenKind::Asterisk {
            self.next_token();

            ty = Ok(AstTypeKind::Pointer(Box::new(AstType {
                kind: ty?,
                span: self.span,
            })));
        }

        self.next_token();

        Ok(AstType {
            kind: ty?,
            span: self.span,
        })
    }
}
