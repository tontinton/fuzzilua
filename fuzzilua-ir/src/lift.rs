use crate::bitset::VarBitset;
use crate::types::{Instruction, Op, Program, Variable};
use std::fmt::Write;

const INDENT: &str = "  ";

pub fn lift(program: &Program) -> String {
    let mut out = String::new();
    let mut depth: usize = 0;
    // VarBitset instead of HashSet<Variable>: same rationale as validate.rs.
    let mut declared = VarBitset::with_capacity(program.next_var);

    for instr in &program.instructions {
        lift_instruction(instr, &mut out, &mut depth, &mut declared);
    }
    out
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str(INDENT);
    }
}

fn declare(out: &mut String, v: Variable, declared: &mut VarBitset) {
    if declared.insert(v) {
        out.push_str("local ");
    }
}

fn join_vars(vars: &[Variable]) -> String {
    vars.iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn emit_assign(
    out: &mut String,
    depth: usize,
    output: Variable,
    declared: &mut VarBitset,
    rhs: std::fmt::Arguments<'_>,
) {
    indent(out, depth);
    declare(out, output, declared);
    writeln!(out, "{output} = {rhs}").unwrap();
}

/// Only emits `local` when ALL vars are new. Mixing local and non-local in
/// the same multi-assign (e.g. `local a, b = ...` where b exists) is invalid Lua.
fn declare_multi_assign(
    out: &mut String,
    depth: usize,
    outputs: &[Variable],
    declared: &mut VarBitset,
    rhs: &str,
) {
    indent(out, depth);
    let all_new = outputs.iter().all(|v| !declared.contains(v));
    if all_new {
        out.push_str("local ");
    }
    let names: Vec<String> = outputs
        .iter()
        .map(|v| {
            declared.insert(*v);
            v.to_string()
        })
        .collect();
    writeln!(out, "{} = {rhs}", names.join(", ")).unwrap();
}

fn escape_lua_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('"');
    for b in s.bytes() {
        match b {
            b'\\' => result.push_str("\\\\"),
            b'"' => result.push_str("\\\""),
            b'\n' => result.push_str("\\n"),
            b'\r' => result.push_str("\\r"),
            b'\t' => result.push_str("\\t"),
            b'\0' => result.push_str("\\0"),
            0x01..=0x1f | 0x7f..=0xff => {
                write!(result, "\\{b}").unwrap();
            }
            _ => result.push(b as char),
        }
    }
    result.push('"');
    result
}

fn end_block(out: &mut String, depth: &mut usize, terminator: &str) {
    *depth = depth.saturating_sub(1);
    indent(out, *depth);
    writeln!(out, "{terminator}").unwrap();
}

fn lift_instruction(
    instr: &Instruction,
    out: &mut String,
    depth: &mut usize,
    declared: &mut VarBitset,
) {
    let inp = &instr.inputs;
    let outp = &instr.outputs;

    match &instr.op {
        Op::LoadNil => {
            emit_assign(out, *depth, outp[0], declared, format_args!("nil"));
        }
        Op::LoadBool(b) => {
            emit_assign(out, *depth, outp[0], declared, format_args!("{b}"));
        }
        Op::LoadInt(n) => {
            emit_assign(out, *depth, outp[0], declared, format_args!("{n}"));
        }
        Op::LoadFloat(n) => {
            let rhs = if n.is_nan() {
                "0/0".to_string()
            } else if n.is_infinite() {
                if n.is_sign_positive() {
                    "1/0".to_string()
                } else {
                    "-1/0".to_string()
                }
            } else {
                format!("{n}")
            };
            emit_assign(out, *depth, outp[0], declared, format_args!("{rhs}"));
        }
        Op::LoadString(s) => {
            let escaped = escape_lua_string(s);
            emit_assign(out, *depth, outp[0], declared, format_args!("{escaped}"));
        }
        Op::Reassign => {
            emit_assign(out, *depth, outp[0], declared, format_args!("{}", inp[0]));
        }
        Op::CreateTable => {
            emit_assign(out, *depth, outp[0], declared, format_args!("{{}}"));
        }
        Op::TableSetField(field) => {
            let key = escape_lua_string(field);
            indent(out, *depth);
            writeln!(out, "{}[{key}] = {}", inp[0], inp[1]).unwrap();
        }
        Op::TableGetField(field) => {
            let key = escape_lua_string(field);
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("{}[{key}]", inp[0]),
            );
        }
        Op::TableSetIndex => {
            indent(out, *depth);
            writeln!(out, "{}[{}] = {}", inp[0], inp[1], inp[2]).unwrap();
        }
        Op::TableGetIndex => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("{}[{}]", inp[0], inp[1]),
            );
        }
        Op::TableSetNumericField(idx) => {
            indent(out, *depth);
            writeln!(out, "{}[{idx}] = {}", inp[0], inp[1]).unwrap();
        }
        Op::TableGetNumericField(idx) => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("{}[{idx}]", inp[0]),
            );
        }
        Op::BeginFunction { param_count } => {
            indent(out, *depth);
            declare(out, outp[0], declared);
            let params: Vec<String> = outp[1..1 + *param_count as usize]
                .iter()
                .map(|v| {
                    declared.insert(*v);
                    v.to_string()
                })
                .collect();
            writeln!(out, "{} = function({})", outp[0], params.join(", ")).unwrap();
            *depth += 1;
        }
        Op::EndFunction | Op::EndIf | Op::EndWhile | Op::EndForIn | Op::EndForRange => {
            end_block(out, depth, "end");
        }
        Op::EndPcall => {
            end_block(out, depth, "end)");
        }
        Op::Return => {
            indent(out, *depth);
            if inp.is_empty() {
                writeln!(out, "return").unwrap();
            } else {
                writeln!(out, "return {}", join_vars(inp)).unwrap();
            }
        }
        Op::CallFunction { arg_count, .. } => {
            let func = inp[0];
            let args = join_vars(&inp[1..1 + *arg_count as usize]);
            if outp.is_empty() {
                indent(out, *depth);
                writeln!(out, "{func}({args})").unwrap();
            } else {
                declare_multi_assign(out, *depth, outp, declared, &format!("{func}({args})"));
            }
        }
        Op::BeginIf => {
            indent(out, *depth);
            writeln!(out, "if {} then", inp[0]).unwrap();
            *depth += 1;
        }
        Op::BeginElse => {
            *depth = depth.saturating_sub(1);
            indent(out, *depth);
            writeln!(out, "else").unwrap();
            *depth += 1;
        }
        Op::BeginWhile => {
            indent(out, *depth);
            writeln!(out, "while {} do", inp[0]).unwrap();
            *depth += 1;
        }
        Op::BeginForIn => {
            indent(out, *depth);
            let iter_vars: Vec<String> = outp
                .iter()
                .map(|v| {
                    declared.insert(*v);
                    v.to_string()
                })
                .collect();
            writeln!(out, "for {} in {} do", iter_vars.join(", "), join_vars(inp)).unwrap();
            *depth += 1;
        }
        Op::BeginForRange => {
            indent(out, *depth);
            declared.insert(outp[0]);
            if inp.len() >= 3 {
                writeln!(
                    out,
                    "for {} = {}, {}, {} do",
                    outp[0], inp[0], inp[1], inp[2]
                )
                .unwrap();
            } else {
                writeln!(out, "for {} = {}, {} do", outp[0], inp[0], inp[1]).unwrap();
            }
            *depth += 1;
        }
        Op::Break => {
            indent(out, *depth);
            writeln!(out, "break").unwrap();
        }
        Op::BinaryOp(bin_op) => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("{} {bin_op} {}", inp[0], inp[1]),
            );
        }
        Op::UnaryOp(un_op) => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("{un_op}{}", inp[0]),
            );
        }
        Op::Compare(cmp_op) => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("{} {cmp_op} {}", inp[0], inp[1]),
            );
        }
        Op::SetMetatable => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("setmetatable({}, {})", inp[0], inp[1]),
            );
        }
        Op::GetMetatable => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("getmetatable({})", inp[0]),
            );
        }
        Op::TypeOf => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("type({})", inp[0]),
            );
        }
        Op::ToNumber => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("tonumber({})", inp[0]),
            );
        }
        Op::ToString => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("tostring({})", inp[0]),
            );
        }
        Op::ToStringFmt(fmt) => {
            let escaped = escape_lua_string(fmt);
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("string.format({escaped}, {})", inp[0]),
            );
        }
        Op::Print => {
            indent(out, *depth);
            writeln!(out, "print({})", join_vars(inp)).unwrap();
        }
        Op::RawGet => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("rawget({}, {})", inp[0], inp[1]),
            );
        }
        Op::RawSet => {
            indent(out, *depth);
            writeln!(out, "rawset({}, {}, {})", inp[0], inp[1], inp[2]).unwrap();
        }
        Op::RawEqual => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("rawequal({}, {})", inp[0], inp[1]),
            );
        }
        Op::Select => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("select({}, {})", inp[0], inp[1]),
            );
        }
        Op::Unpack => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("unpack({})", inp[0]),
            );
        }
        Op::Ipairs => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("ipairs({})", inp[0]),
            );
        }
        Op::Pairs => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("pairs({})", inp[0]),
            );
        }
        Op::Next => {
            if outp.len() >= 2 {
                let second_arg = if inp.len() > 1 {
                    inp[1].to_string()
                } else {
                    "nil".to_string()
                };
                declare_multi_assign(
                    out,
                    *depth,
                    outp,
                    declared,
                    &format!("next({}, {second_arg})", inp[0]),
                );
            } else {
                emit_assign(
                    out,
                    *depth,
                    outp[0],
                    declared,
                    format_args!("next({})", inp[0]),
                );
            }
        }
        Op::SetFenv => {
            indent(out, *depth);
            writeln!(out, "setfenv({}, {})", inp[0], inp[1]).unwrap();
        }
        Op::GetFenv => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("getfenv({})", inp[0]),
            );
        }
        Op::CollectGarbage(mode) => {
            indent(out, *depth);
            writeln!(out, "collectgarbage({mode})").unwrap();
        }
        Op::CoroutineCreate => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("coroutine.create({})", inp[0]),
            );
        }
        Op::CoroutineResume => {
            let args = join_vars(inp);
            if outp.is_empty() {
                indent(out, *depth);
                writeln!(out, "coroutine.resume({args})").unwrap();
            } else {
                declare_multi_assign(
                    out,
                    *depth,
                    outp,
                    declared,
                    &format!("coroutine.resume({args})"),
                );
            }
        }
        Op::CoroutineYield => {
            indent(out, *depth);
            writeln!(out, "coroutine.yield({})", join_vars(inp)).unwrap();
        }
        Op::CoroutineWrap => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("coroutine.wrap({})", inp[0]),
            );
        }
        Op::BeginPcall => {
            indent(out, *depth);
            declare(out, outp[0], declared);
            writeln!(out, "{} = pcall(function()", outp[0]).unwrap();
            *depth += 1;
        }
        Op::StringLen => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("string.len({})", inp[0]),
            );
        }
        Op::StringSub => {
            if inp.len() >= 3 {
                emit_assign(
                    out,
                    *depth,
                    outp[0],
                    declared,
                    format_args!("string.sub({}, {}, {})", inp[0], inp[1], inp[2]),
                );
            } else {
                emit_assign(
                    out,
                    *depth,
                    outp[0],
                    declared,
                    format_args!("string.sub({}, {})", inp[0], inp[1]),
                );
            }
        }
        Op::StringFind => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("string.find({}, {})", inp[0], inp[1]),
            );
        }
        Op::StringFormat(fmt) => {
            let escaped = escape_lua_string(fmt);
            let args = join_vars(inp);
            if args.is_empty() {
                emit_assign(
                    out,
                    *depth,
                    outp[0],
                    declared,
                    format_args!("string.format({escaped})"),
                );
            } else {
                emit_assign(
                    out,
                    *depth,
                    outp[0],
                    declared,
                    format_args!("string.format({escaped}, {args})"),
                );
            }
        }
        Op::StringRep => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("string.rep({}, {})", inp[0], inp[1]),
            );
        }
        Op::StringByte => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("string.byte({})", inp[0]),
            );
        }
        Op::StringChar => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("string.char({})", inp[0]),
            );
        }
        Op::StringGmatch(pattern) => {
            let escaped = escape_lua_string(pattern);
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("string.gmatch({}, {escaped})", inp[0]),
            );
        }
        Op::StringGsub(pattern, replacement) => {
            let p = escape_lua_string(pattern);
            let r = escape_lua_string(replacement);
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("string.gsub({}, {p}, {r})", inp[0]),
            );
        }
        Op::Loadstring => {
            emit_assign(
                out,
                *depth,
                outp[0],
                declared,
                format_args!("loadstring({})", inp[0]),
            );
        }
        Op::Error => {
            indent(out, *depth);
            writeln!(out, "error({})", inp[0]).unwrap();
        }
        Op::TableInsert => {
            indent(out, *depth);
            if inp.len() >= 3 {
                writeln!(out, "table.insert({}, {}, {})", inp[0], inp[1], inp[2]).unwrap();
            } else {
                writeln!(out, "table.insert({}, {})", inp[0], inp[1]).unwrap();
            }
        }
        Op::TableRemove => {
            if inp.len() >= 2 {
                emit_assign(
                    out,
                    *depth,
                    outp[0],
                    declared,
                    format_args!("table.remove({}, {})", inp[0], inp[1]),
                );
            } else {
                emit_assign(
                    out,
                    *depth,
                    outp[0],
                    declared,
                    format_args!("table.remove({})", inp[0]),
                );
            }
        }
        Op::Nop => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_basic_strings() {
        assert_eq!(escape_lua_string("hello"), "\"hello\"");
        assert_eq!(escape_lua_string(""), "\"\"");
    }

    #[test]
    fn escape_special_chars() {
        assert_eq!(escape_lua_string("a\"b"), "\"a\\\"b\"");
        assert_eq!(escape_lua_string("a\\b"), "\"a\\\\b\"");
        assert_eq!(escape_lua_string("a\nb"), "\"a\\nb\"");
        assert_eq!(escape_lua_string("a\0b"), "\"a\\0b\"");
        assert_eq!(escape_lua_string("a\tb"), "\"a\\tb\"");
        assert_eq!(escape_lua_string("a\rb"), "\"a\\rb\"");
    }

    #[test]
    fn escape_non_ascii() {
        let s = escape_lua_string("\x01\x7f");
        assert_eq!(s, "\"\\1\\127\"");
        let s2 = escape_lua_string("é");
        assert!(s2.contains("\\"));
    }
}
