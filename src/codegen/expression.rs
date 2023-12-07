use super::{
    symbol_table::SymbolTable, types::AstType, CompileError, CompileResult, Literal, Statement,
};
use crate::{
    display, BinaryOperator, CodegenType, Compiler, DisplayNode, ExpressionCodegen, Position,
    StatementCodegen, UnaryOperator, Value,
};
use inkwell::values::{BasicMetadataValueEnum, BasicValue};
use std::fmt;

#[derive(Debug, Clone)]
pub enum Expression {
    Literal(Literal),
    Binary(BinaryExpression),
    Unary(UnaryExpression),
    Assign(AssignExpression),
    Block(BlockExpression),
    If(IfExpression),
    Call(CallExpression),
    Index(IndexExpression),
    Typeof(TypeofExpression),
    Sizeof(SizeofExpression),
    Cast(CastExpression),
    Pointer(PointerExpression),
    Dereference(DereferenceExpression),
}

impl ExpressionCodegen for Expression {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        macro_rules! inner {
            ($($ident:ident)*) => {
                match self {
                    $(
                        Expression::$ident(expression) => expression.codegen(compiler),
                    )*
                }
            };
        }
        inner! { Literal Binary Unary Assign Block If Call Index Typeof Sizeof Cast Dereference Pointer }
    }
}

impl From<Expression> for Position {
    fn from(expression: Expression) -> Self {
        macro_rules! inner {
            ($($ident:ident)*) => {
                match expression {
                    Expression::Literal(literal) => Position::from(literal),
                    $(
                        Expression::$ident(expression) => expression.position,
                    )*
                }
            };
        }

        inner! { Binary Unary Assign Block If Call Index Typeof Sizeof Cast Dereference Pointer }
    }
}

impl DisplayNode for Expression {
    fn display(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        macro_rules! inner {
            ($($ident:ident)*) => {
                match self {
                    $(
                        Expression::$ident(expression) => expression.display(f, indent),
                    )*
                }
            };
        }

        inner! { Literal Binary Unary Assign Block If Call Index Typeof Sizeof Cast Dereference Pointer }
    }
}

#[derive(Debug, Clone)]
pub struct BinaryExpression {
    pub left: Box<Expression>,
    pub operator: BinaryOperator,
    pub right: Box<Expression>,
    pub position: Position,
}

impl ExpressionCodegen for BinaryExpression {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        use BinaryOperator::*;
        match self.operator {
            Dot => self.codegen_dot(compiler),
            Plus | Minus | Asterisk | Slash | Percent => self.codegen_arithmetic(compiler),
            EQ | NEQ | LT | GT | LTE | GTE => self.codegen_comparison(compiler),
        }
    }
}

impl BinaryExpression {
    fn codegen_dot<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let left = self.left.codegen(compiler)?;
        let left_ty = match left.ty {
            CodegenType::Struct(struct_type) => struct_type,
            _ => return Err(CompileError::member_access_non_struct_type(self.position)),
        };

        let right = match *self.right.clone() {
            Expression::Literal(Literal::Identifier(identifier)) => identifier,
            _ => return Err(CompileError::expected("identifier", self.position)),
        };

        let field = match left_ty.fields.get(&right.identifier) {
            Some(field) => field,
            None => {
                return Err(CompileError::field_not_found(
                    right.identifier,
                    right.position,
                ))
            }
        };
        let field_ll_ty = field.1.to_llvm_type(compiler.context);

        let ptr = unsafe {
            compiler.builder.build_gep(
                CodegenType::Struct(left_ty.clone()).to_llvm_type(compiler.context),
                left.llvm_value.into_pointer_value(),
                &[compiler.context.i64_type().const_int(field.0 as u64, false)],
                format!("ptr.struct.{}.{}", left_ty.name, field.0).as_str(),
            )
        };

        let load = compiler.builder.build_load(
            field_ll_ty,
            ptr,
            format!("struct.{}.{}", left_ty.name, field.0).as_str(),
        );

        Ok(Value::new(
            load,
            left_ty.fields.get(&right.identifier).unwrap().1.clone(),
        ))
    }

    fn codegen_arithmetic<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let left = self.left.codegen(compiler)?;
        let right = self.right.codegen(compiler)?;

        let left = match left.ty {
            CodegenType::Int => left.llvm_value.into_int_value(),
            _ => {
                return Err(CompileError::expected(
                    "int",
                    Position::from(*self.left.clone()),
                ))
            }
        };

        let right = match right.ty {
            CodegenType::Int => right.llvm_value.into_int_value(),
            _ => {
                return Err(CompileError::expected(
                    "int",
                    Position::from(*self.right.clone()),
                ))
            }
        };

        use BinaryOperator::*;

        let result = match self.operator {
            Plus => compiler.builder.build_int_add(left, right, "add"),
            Minus => compiler.builder.build_int_sub(left, right, "sub"),
            Asterisk => compiler.builder.build_int_mul(left, right, "mul"),
            Slash => compiler.builder.build_int_signed_div(left, right, "div"),
            Percent => compiler.builder.build_int_signed_rem(left, right, "rem"),
            _ => unreachable!(),
        };

        Ok(Value::new(result.into(), CodegenType::Int))
    }

    fn codegen_comparison<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let left = self.left.codegen(compiler)?;
        let right = self.right.codegen(compiler)?;

        let left = match left.ty {
            CodegenType::Int => left.llvm_value.into_int_value(),
            _ => {
                return Err(CompileError::expected(
                    "int",
                    Position::from(*self.left.clone()),
                ))
            }
        };

        let right = match right.ty {
            CodegenType::Int => right.llvm_value.into_int_value(),
            _ => {
                return Err(CompileError::expected(
                    "int",
                    Position::from(*self.right.clone()),
                ))
            }
        };

        use BinaryOperator::*;

        let result = match self.operator {
            EQ => compiler
                .builder
                .build_int_compare(inkwell::IntPredicate::EQ, left, right, "eq"),
            NEQ => compiler
                .builder
                .build_int_compare(inkwell::IntPredicate::NE, left, right, "ne"),
            LT => compiler
                .builder
                .build_int_compare(inkwell::IntPredicate::SLT, left, right, "lt"),
            GT => compiler
                .builder
                .build_int_compare(inkwell::IntPredicate::SGT, left, right, "gt"),
            LTE => {
                compiler
                    .builder
                    .build_int_compare(inkwell::IntPredicate::SLE, left, right, "lte")
            }
            GTE => {
                compiler
                    .builder
                    .build_int_compare(inkwell::IntPredicate::SGE, left, right, "gte")
            }
            _ => unreachable!(),
        };

        Ok(Value::new(result.into(), CodegenType::Boolean))
    }
}

impl DisplayNode for BinaryExpression {
    fn display(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        self.left.display(f, indent)?;
        match self.operator.clone() {
            BinaryOperator::Dot => write!(f, ".")?,
            operator => write!(f, " {} ", operator)?,
        }
        self.right.display(f, indent)
    }
}

#[derive(Debug, Clone)]
pub struct UnaryExpression {
    pub operator: UnaryOperator,
    pub expression: Box<Expression>,
    pub position: Position,
}

impl ExpressionCodegen for UnaryExpression {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        use UnaryOperator::*;
        match self.operator {
            Minus => self.codegen_minus(compiler),
            Not => self.codegen_not(compiler),
        }
    }
}

impl UnaryExpression {
    fn codegen_minus<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let expression = self.expression.codegen(compiler)?;

        Ok(match expression.ty {
            CodegenType::Int => {
                let result = compiler
                    .builder
                    .build_int_neg(expression.llvm_value.into_int_value(), "neg");
                Value::new(result.into(), CodegenType::Int)
            }
            CodegenType::Float => {
                let result = compiler
                    .builder
                    .build_float_neg(expression.llvm_value.into_float_value(), "neg");
                Value::new(result.into(), CodegenType::Float)
            }
            _ => return Err(CompileError::expected("int or float", self.position)),
        })
    }

    fn codegen_not<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let expression = self.expression.codegen(compiler)?;

        let expression = match expression.ty {
            CodegenType::Boolean => expression.llvm_value.into_int_value(),
            _ => return Err(CompileError::expected("boolean", self.position)),
        };

        let result = compiler.builder.build_not(expression, "not");
        Ok(Value::new(result.into(), CodegenType::Boolean))
    }
}

impl DisplayNode for UnaryExpression {
    fn display(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        write!(f, "{}", self.operator)?;
        self.expression.display(f, indent)
    }
}

#[derive(Debug, Clone)]
pub struct AssignExpression {
    pub expression: Box<Expression>,
    pub value: Box<Expression>,
    pub position: Position,
}

impl ExpressionCodegen for AssignExpression {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let value = self.value.codegen(compiler)?;

        Ok(match *self.expression.clone() {
            Expression::Literal(Literal::Identifier(identifier)) => {
                match compiler.symbol_table.get_variable(&identifier.identifier) {
                    Some(entry) => {
                        if entry.1 != value.ty {
                            return Err(CompileError::type_mismatch(
                                entry.1.clone(),
                                value.ty,
                                identifier.position,
                            ));
                        }

                        compiler.builder.build_store(entry.0, value.llvm_value);
                        value
                    }
                    None => {
                        return Err(CompileError::identifier_not_found(
                            identifier.identifier,
                            self.position,
                        ))
                    }
                }
            }
            Expression::Index(index) => {
                let left = index.left.codegen(compiler)?;
                let index = index.index.codegen(compiler)?;

                match left.ty {
                    CodegenType::Array(array) => {
                        let index = match index.ty {
                            CodegenType::Int => index.llvm_value.into_int_value(),
                            _ => return Err(CompileError::expected("int", self.position)),
                        };
                        let element_ll_ty = array.ty.to_llvm_type(compiler.context);

                        let ptr = unsafe {
                            compiler.builder.build_gep(
                                element_ll_ty,
                                left.llvm_value.into_pointer_value(),
                                &[index],
                                "ptr_array_index",
                            )
                        };

                        if *array.ty != value.ty {
                            return Err(CompileError::type_mismatch(
                                *array.ty,
                                value.ty,
                                self.position,
                            ));
                        }

                        compiler.builder.build_store(ptr, value.llvm_value);
                        value
                    }
                    _ => {
                        return Err(CompileError::type_that_cannot_be_indexed(
                            (*self.expression.clone()).into(),
                        ))
                    }
                }
            }
            Expression::Binary(BinaryExpression {
                left,
                operator: BinaryOperator::Dot,
                right,
                position,
            }) => {
                let left = left.codegen(compiler)?;
                let left_ty = match left.ty {
                    CodegenType::Struct(struct_type) => struct_type,
                    _ => return Err(CompileError::member_access_non_struct_type(position)),
                };

                let right = match *right.clone() {
                    Expression::Literal(Literal::Identifier(identifier)) => identifier,
                    _ => return Err(CompileError::expected("identifier", position)),
                };

                let field = match left_ty.fields.get(&right.identifier) {
                    Some(field) => field,
                    None => {
                        return Err(CompileError::field_not_found(
                            right.identifier,
                            right.position,
                        ))
                    }
                };

                let ptr = unsafe {
                    compiler.builder.build_gep(
                        CodegenType::Struct(left_ty.clone()).to_llvm_type(compiler.context),
                        left.llvm_value.into_pointer_value(),
                        &[compiler.context.i64_type().const_int(field.0 as u64, false)],
                        format!("ptr.struct.{}.{}", left_ty.name, field.0).as_str(),
                    )
                };

                if field.1 != value.ty {
                    return Err(CompileError::type_mismatch(
                        field.1.clone(),
                        value.ty,
                        self.position,
                    ));
                }

                compiler.builder.build_store(ptr, value.llvm_value);
                value
            }
            Expression::Dereference(dereference) => {
                let expression = dereference.expression.codegen(compiler)?;

                match expression.ty {
                    CodegenType::Pointer(ty) => {
                        if *ty != value.ty {
                            return Err(CompileError::type_mismatch(
                                *ty,
                                value.ty,
                                dereference.position,
                            ));
                        }

                        compiler.builder.build_store(
                            expression.llvm_value.into_pointer_value(),
                            value.llvm_value,
                        );
                        value
                    }
                    _ => return Err(CompileError::expected("pointer", self.position)),
                }
            }
            _ => return Err(CompileError::cannot_be_assigned(self.position)),
        })
    }
}

impl DisplayNode for AssignExpression {
    fn display(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        self.expression.display(f, indent)?;
        write!(f, " = ")?;
        self.value.display(f, indent)
    }
}

#[derive(Debug, Clone)]
pub struct BlockExpression {
    pub statements: Vec<Statement>,
    pub position: Position,
}

impl ExpressionCodegen for BlockExpression {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let original_symbol_table = compiler.symbol_table.clone();
        compiler.symbol_table = SymbolTable::new_with_parent(compiler.symbol_table.clone());

        for statement in self.statements.clone() {
            if let Statement::Return(return_statement) = statement {
                let value = return_statement.value.codegen(compiler)?;
                compiler.symbol_table = original_symbol_table;
                return Ok(value);
            }

            statement.codegen(compiler)?;
        }

        compiler.symbol_table = original_symbol_table;

        Ok(Value::new(
            compiler.context.i64_type().const_int(0, false).into(),
            CodegenType::Void,
        ))
    }
}

impl DisplayNode for BlockExpression {
    fn display(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        writeln!(f, "{{")?;
        for statement in self.statements.clone() {
            statement.display(f, indent + 1)?;
        }
        display::indent(f, indent)?;
        write!(f, "}}")
    }
}

#[derive(Debug, Clone)]
pub struct IfExpression {
    pub condition: Box<Expression>,
    pub consequence: BlockExpression,
    pub alternative: Option<BlockExpression>,
    pub position: Position,
}

impl ExpressionCodegen for IfExpression {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let condition = self.condition.codegen(compiler)?;
        if condition.ty != CodegenType::Boolean {
            return Err(CompileError::expected("boolean", self.position));
        }

        let function = compiler
            .builder
            .get_insert_block()
            .unwrap()
            .get_parent()
            .unwrap();

        let then_block = compiler.context.append_basic_block(function, "then");
        let else_block = compiler.context.append_basic_block(function, "else");
        let merge_block = compiler.context.append_basic_block(function, "merge");

        compiler.builder.build_conditional_branch(
            condition.llvm_value.into_int_value(),
            then_block,
            else_block,
        );

        compiler.builder.position_at_end(then_block);

        let then = self.consequence.codegen(compiler)?;
        compiler.builder.build_unconditional_branch(merge_block);

        let then_block = compiler.builder.get_insert_block().unwrap();

        compiler.builder.position_at_end(else_block);

        let else_ = match self.alternative.clone() {
            Some(expr) => {
                let else_ = expr.codegen(compiler)?;

                if then.ty != else_.ty {
                    return Err(CompileError::type_mismatch(
                        then.ty,
                        else_.ty,
                        self.position,
                    ));
                }

                else_
            }
            None => Value::new(
                compiler
                    .context
                    .i64_type()
                    .const_int(0, false)
                    .as_basic_value_enum(),
                CodegenType::Void,
            ),
        };
        compiler.builder.build_unconditional_branch(merge_block);

        let else_block = compiler.builder.get_insert_block().unwrap();

        compiler.builder.position_at_end(merge_block);

        let phi = compiler
            .builder
            .build_phi(then.llvm_value.get_type(), "iftmp");
        phi.add_incoming(&[
            (&then.llvm_value, then_block),
            (&else_.llvm_value, else_block),
        ]);

        Ok(Value::new(phi.as_basic_value(), then.ty))
    }
}

impl DisplayNode for IfExpression {
    fn display(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        write!(f, "if ")?;
        self.condition.display(f, indent)?;
        write!(f, " ")?;
        self.consequence.display(f, indent)?;
        if let Some(alternative) = self.alternative.clone() {
            write!(f, " else ")?;
            alternative.display(f, indent)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CallExpression {
    pub function: Box<Expression>,
    pub arguments: Vec<Expression>,
    pub position: Position,
}

impl ExpressionCodegen for CallExpression {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let (function, entry) = match *self.function.clone() {
            Expression::Literal(Literal::Identifier(identifier)) => {
                let function = match compiler.symbol_table.get_function(&identifier.identifier) {
                    Some(entry) => entry,
                    None => {
                        return Err(CompileError::function_not_found(
                            identifier.identifier,
                            identifier.position,
                        ))
                    }
                };
                let value = match compiler.module.get_function(&identifier.identifier) {
                    Some(value) => value,
                    None => {
                        return Err(CompileError::function_not_found(
                            identifier.identifier,
                            identifier.position,
                        ))
                    }
                };

                (value, function)
            }
            _ => return Err(CompileError::call_non_function_type(self.position)),
        };

        if self.arguments.len() != entry.1.parameters.len() {
            return Err(CompileError::wrong_number_of_arguments(
                entry.1.parameters.len(),
                self.arguments.len(),
                self.position,
            ));
        }

        let mut arguments: Vec<BasicMetadataValueEnum> = Vec::new();

        for argument in self.arguments.clone() {
            let value = argument.codegen(compiler)?;
            arguments.push(value.llvm_value.into());

            let paramter_ty = entry.1.parameters[arguments.len() - 1].clone();
            if value.ty != paramter_ty {
                return Err(CompileError::type_mismatch(
                    paramter_ty,
                    value.ty,
                    argument.into(),
                ));
            }
        }

        Ok(
            match compiler
                .builder
                .build_call(function, arguments.as_slice(), "call")
                .try_as_basic_value()
                .left()
            {
                Some(value) => Value::new(value, *entry.1.return_type.clone()),
                None => Value::new(
                    compiler.context.i64_type().const_int(0, false).into(),
                    CodegenType::Int, // Void
                ),
            },
        )
    }
}

impl DisplayNode for CallExpression {
    fn display(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        self.function.display(f, indent)?;
        write!(f, "(")?;
        for (i, argument) in self.arguments.iter().enumerate() {
            if i != 0 {
                write!(f, ", ")?;
            }
            argument.display(f, indent)?;
        }
        write!(f, ")")
    }
}

#[derive(Debug, Clone)]
pub struct IndexExpression {
    pub left: Box<Expression>,
    pub index: Box<Expression>,
    pub position: Position,
}

impl ExpressionCodegen for IndexExpression {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let left = self.left.codegen(compiler)?;
        let index = self.index.codegen(compiler)?;

        Ok(match left.ty {
            CodegenType::Array(array) => {
                let index = match index.ty {
                    CodegenType::Int => index.llvm_value.into_int_value(),
                    _ => return Err(CompileError::expected("int", self.position)),
                };
                let element_ll_ty = array.ty.to_llvm_type(compiler.context);

                let ptr = unsafe {
                    compiler.builder.build_gep(
                        element_ll_ty,
                        left.llvm_value.into_pointer_value(),
                        &[index],
                        "ptr_array_index",
                    )
                };

                Value::new(
                    compiler
                        .builder
                        .build_load(element_ll_ty, ptr, "load_array_index"),
                    *array.ty,
                )
            }
            _ => {
                return Err(CompileError::type_that_cannot_be_indexed(
                    (*self.left.clone()).into(),
                ))
            }
        })
    }
}

impl DisplayNode for IndexExpression {
    fn display(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        self.left.display(f, indent)?;
        write!(f, "[")?;
        self.index.display(f, indent)?;
        write!(f, "]")
    }
}

#[derive(Debug, Clone)]
pub struct TypeofExpression {
    pub expression: Box<Expression>,
    pub position: Position,
}

impl ExpressionCodegen for TypeofExpression {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let expr = self.expression.codegen(compiler)?;

        use CodegenType::*;

        let ty_num = match expr.ty {
            Int => 0,
            Float => 1,
            String => 2,
            Boolean => 3,
            Array(_) => 4,
            Struct(_) => 5,
            Function(_) => 6,
            Void => 7,
            Pointer(_) => 8,
        };

        Ok(Value::new(
            compiler
                .context
                .i64_type()
                .const_int(ty_num as u64, false)
                .into(),
            CodegenType::Int,
        ))
    }
}

impl DisplayNode for TypeofExpression {
    fn display(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        write!(f, "typeof ")?;
        self.expression.display(f, indent)
    }
}

#[derive(Debug, Clone)]
pub struct SizeofExpression {
    pub expression: Box<Expression>,
    pub position: Position,
}

impl ExpressionCodegen for SizeofExpression {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let value = self.expression.codegen(compiler)?;

        let size = match value.ty {
            CodegenType::Array(array_type) => {
                let length = match array_type.len {
                    Some(length) => length,
                    _ => return Err(CompileError::unknown_size(array_type.position)),
                };
                let length = compiler.context.i64_type().const_int(length as u64, false);

                compiler.builder.build_int_mul(
                    array_type.ty.size_of(compiler.context, self.position)?,
                    length,
                    "array_size",
                )
            }
            ty => ty.size_of(compiler.context, self.position)?,
        };

        Ok(Value::new(size.as_basic_value_enum(), CodegenType::Int))
    }
}

impl DisplayNode for SizeofExpression {
    fn display(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        write!(f, "sizeof ")?;
        self.expression.display(f, indent)
    }
}

#[derive(Debug, Clone)]
pub struct CastExpression {
    pub expression: Box<Expression>,
    pub cast_ty: AstType,
    pub position: Position,
}

impl ExpressionCodegen for CastExpression {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let value = self.expression.codegen(compiler)?;
        let ty = self.cast_ty.kind.to_codegen_type(&compiler.symbol_table)?;

        let result = match ty {
            CodegenType::Int => match value.ty {
                CodegenType::Float => compiler
                    .builder
                    .build_float_to_signed_int(
                        value.llvm_value.into_float_value(),
                        compiler.context.i64_type(),
                        "cast",
                    )
                    .as_basic_value_enum(),
                CodegenType::Boolean => compiler
                    .builder
                    .build_int_z_extend(
                        value.llvm_value.into_int_value(),
                        compiler.context.i64_type(),
                        "cast",
                    )
                    .as_basic_value_enum(),
                CodegenType::Pointer(_) => compiler
                    .builder
                    .build_ptr_to_int(
                        value.llvm_value.into_pointer_value(),
                        compiler.context.i64_type(),
                        "cast",
                    )
                    .as_basic_value_enum(),
                _ => return Err(CompileError::expected("float", self.position)),
            },
            CodegenType::Float => match value.ty {
                CodegenType::Int => compiler
                    .builder
                    .build_signed_int_to_float(
                        value.llvm_value.into_int_value(),
                        compiler.context.f64_type(),
                        "cast",
                    )
                    .as_basic_value_enum(),
                _ => return Err(CompileError::expected("int", self.position)),
            },
            CodegenType::Pointer(_) => match value.ty {
                CodegenType::Int => compiler
                    .builder
                    .build_int_to_ptr(
                        value.llvm_value.into_int_value(),
                        ty.to_llvm_type(compiler.context).into_pointer_type(),
                        "cast",
                    )
                    .as_basic_value_enum(),
                _ => return Err(CompileError::expected("int", self.position)),
            },
            _ => return Err(CompileError::expected("int or float", self.position)),
        };

        Ok(Value::new(result, ty))
    }
}

impl DisplayNode for CastExpression {
    fn display(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        self.expression.display(f, indent)?;
        write!(f, " as {}", self.cast_ty.kind)
    }
}

#[derive(Debug, Clone)]
pub struct PointerExpression {
    pub expression: Box<Expression>,
    pub position: Position,
}

impl ExpressionCodegen for PointerExpression {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        match *self.expression.clone() {
            Expression::Literal(Literal::Identifier(identifier)) => {
                let value = match compiler.symbol_table.get_variable(&identifier.identifier) {
                    Some(entry) => entry,
                    None => {
                        return Err(CompileError::identifier_not_found(
                            identifier.identifier,
                            identifier.position,
                        ))
                    }
                };

                Ok(Value::new(
                    value.0.as_basic_value_enum(),
                    CodegenType::Pointer(Box::new(value.1.clone())),
                ))
            }
            Expression::Index(index) => {
                let value = index.left.codegen(compiler)?;
                let index = index.index.codegen(compiler)?;

                match value.ty {
                    CodegenType::Array(array) => {
                        let index = match index.ty {
                            CodegenType::Int => index.llvm_value.into_int_value(),
                            _ => return Err(CompileError::expected("int", self.position)),
                        };
                        let element_ll_ty = array.ty.to_llvm_type(compiler.context);

                        let ptr = unsafe {
                            compiler.builder.build_gep(
                                element_ll_ty,
                                value.llvm_value.into_pointer_value(),
                                &[index],
                                "ptr_array_index",
                            )
                        };

                        Ok(Value::new(
                            ptr.as_basic_value_enum(),
                            CodegenType::Pointer(array.ty),
                        ))
                    }
                    _ => Err(CompileError::type_that_cannot_be_indexed(
                        (*self.expression.clone()).into(),
                    )),
                }
            }
            Expression::Binary(BinaryExpression {
                left,
                operator: BinaryOperator::Dot,
                right,
                position,
            }) => {
                let left = left.codegen(compiler)?;
                let left_ty = match left.ty {
                    CodegenType::Struct(struct_type) => struct_type,
                    _ => return Err(CompileError::member_access_non_struct_type(position)),
                };

                let right = match *right.clone() {
                    Expression::Literal(Literal::Identifier(identifier)) => identifier,
                    _ => return Err(CompileError::expected("identifier", position)),
                };

                let field = match left_ty.fields.get(&right.identifier) {
                    Some(field) => field,
                    None => {
                        return Err(CompileError::field_not_found(
                            right.identifier,
                            right.position,
                        ))
                    }
                };

                let ptr = unsafe {
                    compiler.builder.build_gep(
                        CodegenType::Struct(left_ty.clone()).to_llvm_type(compiler.context),
                        left.llvm_value.into_pointer_value(),
                        &[compiler.context.i64_type().const_int(field.0 as u64, false)],
                        format!("ptr.struct.{}.{}", left_ty.name, field.0).as_str(),
                    )
                };

                Ok(Value::new(
                    ptr.as_basic_value_enum(),
                    CodegenType::Pointer(Box::new(field.1.clone())),
                ))
            }
            expression => {
                let value = expression.codegen(compiler)?;

                Ok(Value::new(
                    compiler
                        .builder
                        .build_alloca(value.ty.to_llvm_type(compiler.context), "ptr")
                        .as_basic_value_enum(),
                    CodegenType::Pointer(Box::new(value.ty)),
                ))
            }
        }
    }
}

impl DisplayNode for PointerExpression {
    fn display(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        write!(f, "&")?;
        self.expression.display(f, indent)
    }
}

#[derive(Debug, Clone)]
pub struct DereferenceExpression {
    pub expression: Box<Expression>,
    pub position: Position,
}

impl ExpressionCodegen for DereferenceExpression {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let value = self.expression.codegen(compiler)?;

        match value.ty {
            CodegenType::Pointer(ty) => {
                let value = compiler.builder.build_load(
                    ty.to_llvm_type(compiler.context),
                    value.llvm_value.into_pointer_value(),
                    "deref",
                );
                Ok(Value::new(value, *ty))
            }
            _ => Err(CompileError::expected("pointer", self.position)),
        }
    }
}

impl DisplayNode for DereferenceExpression {
    fn display(&self, f: &mut fmt::Formatter<'_>, indent: usize) -> fmt::Result {
        write!(f, "*")?;
        self.expression.display(f, indent)
    }
}
