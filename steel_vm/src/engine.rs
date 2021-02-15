use std::{collections::HashMap, convert::TryFrom, io::Read, path::Path, rc::Rc};

use crate::{evaluation_progress::Callback, vm::VirtualMachineCore};
use steel::{
    core::instructions::DenseInstruction,
    gc::Gc,
    parser::ast::ExprKind,
    parser::parser::{ParseError, Parser},
    primitives::ListOperations,
    rerrs::SteelErr,
    rvals::{Result, SteelVal},
    steel_compiler::{compiler::Compiler, constants::ConstantMap, program::Program},
    throw,
};

#[macro_export]
macro_rules! build_engine {

    ($($type:ty),* $(,)?) => {
        {
            let mut interpreter = Engine::new();
            $ (
                interpreter.register_values(<$type>::generate_bindings());
            ) *
            interpreter
        }
    };

    (Structs => {$($type:ty),* $(,)?} Functions => {$($binding:expr => $func:expr),* $(,)?}) => {
        {
            let mut interpreter = Engine::new();
            $ (
                interpreter.register_values(<$type>::generate_bindings());
            ) *

            $ (
                interpreter.register_value($binding.to_string().as_str(), SteelVal::FuncV($func));
            ) *

            interpreter
        }
    };
}

pub struct Engine {
    virtual_machine: VirtualMachineCore,
    compiler: Compiler,
}

impl Engine {
    pub fn new() -> Self {
        Engine {
            virtual_machine: VirtualMachineCore::new(),
            compiler: Compiler::default(),
        }
    }

    pub fn new_with_meta() -> Engine {
        let mut vm = Engine::new();
        vm.register_value("*env*", steel::env::Env::constant_env_to_hashmap());
        vm
    }

    pub fn emit_program(&mut self, expr: &str) -> Result<Program> {
        self.compiler.compile_program(expr)
    }

    pub fn execute(
        &mut self,
        bytecode: Rc<[DenseInstruction]>,
        constant_map: &ConstantMap,
    ) -> Result<Gc<SteelVal>> {
        self.virtual_machine.execute(bytecode, constant_map)
    }

    pub fn emit_instructions(&mut self, exprs: &str) -> Result<Vec<Vec<DenseInstruction>>> {
        self.compiler.emit_instructions(exprs)
    }

    pub fn execute_program(&mut self, program: Program) -> Result<Vec<Gc<SteelVal>>> {
        self.virtual_machine.execute_program(program)
    }

    pub fn register_value(&mut self, name: &str, value: SteelVal) {
        let idx = self.compiler.register(name);
        self.virtual_machine.insert_binding(idx, value);
    }

    pub fn register_gc_value(&mut self, name: &str, value: Gc<SteelVal>) {
        let idx = self.compiler.register(name);
        self.virtual_machine.insert_gc_binding(idx, value);
    }

    pub fn register_values(&mut self, values: Vec<(String, SteelVal)>) {
        for (name, value) in values {
            self.register_value(name.as_str(), value);
        }
    }

    pub fn on_progress(&mut self, callback: Callback) {
        self.virtual_machine.on_progress(callback);
    }

    pub fn extract_value(&self, name: &str) -> Result<SteelVal> {
        let idx = self.compiler.get_idx(name).ok_or_else(throw!(
            Generic => format!("free identifier: {} - identifier given cannot be found in the global environment", name)
        ))?;

        self.virtual_machine.extract_value(idx)
            .ok_or_else(throw!(
                Generic => format!("free identifier: {} - identifier given cannot be found in the global environment", name)
            ))
    }

    pub fn parse_and_execute_without_optimizations(
        &mut self,
        expr: &str,
    ) -> Result<Vec<Gc<SteelVal>>> {
        let program = self.compiler.compile_program(expr)?;
        self.virtual_machine.execute_program(program)
    }

    pub fn parse_and_execute(&mut self, expr: &str) -> Result<Vec<Gc<SteelVal>>> {
        self.parse_and_execute_without_optimizations(expr)
    }

    // Read in the file from the given path and execute accordingly
    // Loads all the functions in from the given env
    pub fn parse_and_execute_from_path<P: AsRef<Path>>(
        &mut self,
        path: P,
    ) -> Result<Vec<Gc<SteelVal>>> {
        let mut file = std::fs::File::open(path)?;
        let mut exprs = String::new();
        file.read_to_string(&mut exprs)?;
        self.parse_and_execute(exprs.as_str())
    }

    // TODO come back to this please

    pub fn parse_and_execute_with_optimizations(
        &mut self,
        expr: &str,
    ) -> Result<Vec<Gc<SteelVal>>> {
        let mut results = Vec::new();
        let mut intern = HashMap::new();

        let parsed: std::result::Result<Vec<ExprKind>, ParseError> =
            Parser::new(expr, &mut intern).collect();
        let parsed = parsed?;

        let expanded_statements = self.compiler.expand_expressions(parsed)?;

        let statements_without_structs = self
            .compiler
            .extract_structs(expanded_statements, &mut results)?;

        let exprs_post_optimization = Self::optimize_exprs(statements_without_structs)?;

        let compiled_instructions = self
            .compiler
            .generate_dense_instructions(exprs_post_optimization, results)?;

        let program = Program::new(
            compiled_instructions,
            (&self.compiler.constant_map).to_bytes()?,
        );

        self.virtual_machine.execute_program(program)
    }

    // TODO come back to this
    pub fn optimize_exprs<I: IntoIterator<Item = ExprKind>>(
        exprs: I,
        // ctx: &mut Ctx<ConstantMap>,
    ) -> Result<Vec<ExprKind>> {
        // println!("About to optimize the input program");

        let converted: Result<Vec<_>> = exprs.into_iter().map(|x| SteelVal::try_from(x)).collect();

        // let converted = Gc::new(SteelVal::try_from(v[0].clone())?);
        let exprs = ListOperations::built_in_list_func_flat_non_gc(converted?)?;

        let mut vm = Engine::new_with_meta();
        vm.parse_and_execute_without_optimizations(steel::stdlib::PRELUDE)?;
        vm.register_gc_value("*program*", exprs);
        let output = vm.parse_and_execute_without_optimizations(steel::stdlib::COMPILER)?;

        // println!("{:?}", output.last().unwrap());

        // if output.len()  1 {
        //     stop!(Generic => "panic! internal compiler error: output did not return a valid program");
        // }

        // TODO
        SteelVal::iter(Gc::clone(output.last().unwrap()))
            .into_iter()
            .map(|x| {
                ExprKind::try_from(x.as_ref()).map_err(|x| SteelErr::Generic(x.to_string(), None))
            })
            .collect::<Result<Vec<ExprKind>>>()
    }
}