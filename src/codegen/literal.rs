use super::{CompileError, CompileResult, Expression};
use crate::{ArrayType, CodegenType, Compiler, ExpressionCodegen, Position, StructType, Value};
use inkwell::{
    types::BasicType,
    values::{BasicValue, BasicValueEnum},
};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub enum Literal {
    Identifier(Identifier),
    Int(IntLiteral),
    Float(FloatLiteral),
    Boolean(BooleanLiteral),
    String(StringLiteral),
    Array(ArrayLiteral),
    Struct(StructLiteral),
}

impl ExpressionCodegen for Literal {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        macro_rules! inner {
            ($($ident:ident)*) => {
                match self {
                    $(
                        Literal::$ident(literal) => literal.codegen(compiler),
                    )*
                }
            };
        }
        inner! { Identifier Int Float Boolean String Array Struct }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct Identifier {
    pub identifier: String,
    pub position: Position,
}

impl ExpressionCodegen for Identifier {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let _symbol_table = compiler.symbol_table.clone();
        let (ptr, ty) = match _symbol_table.get_variable(&self.identifier) {
            Some(entry) => entry,
            None => {
                return Err(CompileError::identifier_not_found(
                    self.identifier.clone(),
                    self.position,
                ))
            }
        };

        Ok(Value::new(
            compiler.builder.build_load(
                ty.to_llvm_type(compiler.context),
                ptr,
                format!("load.{}", self.identifier).as_str(),
            ),
            ty.clone(),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct IntLiteral {
    pub value: i64,
    pub position: Position,
}

impl ExpressionCodegen for IntLiteral {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        Ok(Value::new(
            compiler
                .context
                .i64_type()
                .const_int(self.value as u64, false)
                .into(),
            CodegenType::Int,
        ))
    }
}

#[derive(Debug, Clone)]
pub struct FloatLiteral {
    pub value: f64,
    pub position: Position,
}

impl ExpressionCodegen for FloatLiteral {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        Ok(Value::new(
            compiler.context.f64_type().const_float(self.value).into(),
            CodegenType::Float,
        ))
    }
}

#[derive(Debug, Clone)]
pub struct BooleanLiteral {
    pub value: bool,
    pub position: Position,
}

impl ExpressionCodegen for BooleanLiteral {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        Ok(Value::new(
            compiler
                .context
                .bool_type()
                .const_int(self.value as u64, false)
                .into(),
            CodegenType::Boolean,
        ))
    }
}

#[derive(Debug, Clone)]
pub struct StringLiteral {
    pub value: String,
    pub position: Position,
}

impl ExpressionCodegen for StringLiteral {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let string = compiler
            .builder
            .build_global_string_ptr(self.value.as_str(), ".str");

        Ok(Value::new(
            string.as_basic_value_enum(),
            CodegenType::String,
        ))
    }
}

#[derive(Debug, Clone)]
pub struct ArrayLiteral {
    pub elements: Vec<Element>,
    pub position: Position,
}

#[derive(Debug, Clone)]
pub struct Element {
    pub value: Expression,
    pub position: Position,
}

impl ExpressionCodegen for ArrayLiteral {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let mut values: Vec<BasicValueEnum> = Vec::new();
        let mut element_type = None;

        for val in self.elements.clone() {
            let value = val.value.codegen(compiler)?;
            values.push(value.llvm_value);
            element_type = Some(value.ty);
        }

        let array_type = match element_type {
            Some(ty) => ty,
            None => {
                return Err(CompileError::array_must_have_at_least_one_element(
                    self.position,
                ))
            }
        };

        let array = array_type
            .to_llvm_type(compiler.context)
            .array_type(values.len() as u32);
        let ptr = compiler
            .builder
            .build_alloca(array, ".array")
            .as_basic_value_enum()
            .into_pointer_value();

        for (i, val) in values.iter().enumerate() {
            let field = unsafe {
                compiler.builder.build_in_bounds_gep(
                    compiler.context.i64_type(),
                    ptr,
                    &[compiler.context.i64_type().const_int(i as u64, false)],
                    format!("ptr.array.{}", i).as_str(),
                )
            };
            compiler.builder.build_store(field, *val);
        }

        Ok(Value::new(
            ptr.as_basic_value_enum(),
            CodegenType::Array(ArrayType {
                ty: Box::new(array_type),
                len: Some(values.len()),
                position: self.position,
            }),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct StructLiteral {
    pub name: Identifier,
    pub fields: BTreeMap<String, Field>,
    pub position: Position,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub value: Expression,
    pub position: Position,
}

impl ExpressionCodegen for StructLiteral {
    fn codegen<'a>(&self, compiler: &mut Compiler<'a>) -> CompileResult<Value<'a>> {
        let _symbol_table = compiler.symbol_table.clone();
        let (llvm_struct_type, struct_type) = match _symbol_table.get_struct(&self.name.identifier)
        {
            Some(entry) => entry,
            None => {
                return Err(CompileError::struct_not_found(
                    self.name.identifier.clone(),
                    self.position,
                ))
            }
        };

        let mut values: Vec<BasicValueEnum> = Vec::new();
        let mut feilds_type = BTreeMap::new();

        for (i, val) in self.fields.iter().enumerate() {
            let value = val.1.value.codegen(compiler)?;
            values.push(value.llvm_value);

            let field_type = match struct_type.fields.get(val.0) {
                Some((_, ty)) => ty.clone(),
                None => return Err(CompileError::field_not_found(val.0.clone(), self.position)),
            };

            if field_type != value.ty {
                return Err(CompileError::type_mismatch(
                    field_type,
                    value.ty,
                    self.position,
                ));
            }

            feilds_type.insert(val.0.clone(), (i, value.ty));
        }

        let ptr = compiler
            .builder
            .build_alloca(
                llvm_struct_type,
                format!("struct.{}", self.name.identifier).as_str(),
            )
            .as_basic_value_enum()
            .into_pointer_value();

        for (i, val) in values.iter().enumerate() {
            let field = unsafe {
                compiler.builder.build_in_bounds_gep(
                    compiler.context.i64_type(),
                    ptr,
                    &[compiler.context.i64_type().const_int(i as u64, false)],
                    format!("ptr.struct.{}.{i}", self.name.identifier).as_str(),
                )
            };
            compiler.builder.build_store(field, *val);
        }

        Ok(Value::new(
            ptr.as_basic_value_enum(),
            CodegenType::Struct(StructType {
                name: self.name.identifier.clone(),
                fields: feilds_type,
                position: self.position,
            }),
        ))
    }
}
