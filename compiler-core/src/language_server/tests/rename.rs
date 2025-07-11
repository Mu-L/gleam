use std::collections::HashMap;

use lsp_types::{
    Position, Range, RenameParams, TextDocumentPositionParams, Url, WorkDoneProgressParams,
};

use crate::language_server::tests::{TestProject, find_position_of};

use super::hover;

fn rename(
    tester: &TestProject<'_>,
    new_name: &str,
    position: Position,
) -> Option<(Range, lsp_types::WorkspaceEdit)> {
    let prepare_rename_response = tester.at(position, |engine, params, _| {
        let params = TextDocumentPositionParams {
            text_document: params.text_document,
            position,
        };
        engine.prepare_rename(params).result.unwrap()
    })?;

    let lsp_types::PrepareRenameResponse::Range(range) = prepare_rename_response else {
        return None;
    };

    let edit = tester.at(position, |engine, params, _| {
        let params = RenameParams {
            text_document_position: TextDocumentPositionParams {
                text_document: params.text_document,
                position,
            },
            new_name: new_name.to_string(),
            work_done_progress_params: WorkDoneProgressParams::default(),
        };
        engine.rename(params).result.unwrap()
    })?;

    Some((range, edit))
}

fn apply_rename(
    tester: &TestProject<'_>,
    new_name: &str,
    position: Position,
) -> (Range, HashMap<String, String>) {
    let (range, edit) = rename(tester, new_name, position).expect("Rename failed");
    let changes = edit.changes.expect("No text edit found");
    (range, apply_code_edit(tester, changes))
}

fn apply_code_edit(
    tester: &TestProject<'_>,
    changes: HashMap<Url, Vec<lsp_types::TextEdit>>,
) -> HashMap<String, String> {
    let mut modules = HashMap::new();
    for (uri, change) in changes {
        let module_name = tester.module_name_from_url(&uri).expect("Valid uri");
        let module_code = tester.src_from_module_url(&uri).expect("Module exists");

        _ = modules.insert(module_name, super::apply_code_edit(module_code, change));
    }

    modules
}

macro_rules! assert_rename {
    ($code:literal, $new_name:literal, $position:expr $(,)?) => {
        assert_rename!(TestProject::for_source($code), $new_name, $position);
    };

    (($module_name:literal, $module_src:literal), $code:literal, $new_name:literal, $position:expr $(,)?) => {
        assert_rename!(
            TestProject::for_source($code).add_module($module_name, $module_src),
            $new_name,
            $position
        );
    };

    ($project:expr, $new_name:literal, $position:expr $(,)?) => {
        let project = $project;
        let src = project.src;
        let position = $position.find_position(src);
        let (range, result) = apply_rename(&project, $new_name, position);

        let mut output = String::from("----- BEFORE RENAME\n");
        for (name, src) in project.root_package_modules.iter() {
            output.push_str(&format!("-- {name}.gleam\n{src}\n\n"));
        }
        output.push_str(&format!(
            "-- app.gleam\n{}\n\n----- AFTER RENAME\n",
            hover::show_hover(src, range, range.start)
        ));
        for (name, src) in project.root_package_modules.iter() {
            output.push_str(&format!(
                "-- {name}.gleam\n{}\n\n",
                result
                    .get(*name)
                    .map(|string| string.as_str())
                    .unwrap_or(*src)
            ));
        }
        output.push_str(&format!(
            "-- app.gleam\n{}",
            result
                .get("app")
                .map(|string| string.as_str())
                .unwrap_or(src)
        ));

        insta::assert_snapshot!(insta::internals::AutoName, output, src);
    };
}

macro_rules! assert_no_rename {
    ($code:literal, $new_name:literal, $position:expr $(,)?) => {
        let project = TestProject::for_source($code);
        assert_no_rename!(&project, $new_name, $position);
    };

    ($project:expr, $new_name:literal, $position:expr $(,)?) => {
        let src = $project.src;
        let position = $position.find_position(src);
        let result = rename($project, $new_name, position);
        assert_eq!(result, None);
    };
}

#[test]
fn rename_local_variable() {
    assert_rename!(
        "
pub fn main() {
  let wibble = 10
  wibble
}
",
        "wobble",
        find_position_of("wibble").nth_occurrence(2),
    );
}

#[test]
fn rename_shadowed_local_variable() {
    assert_rename!(
        "
pub fn main() {
  let wibble = 10
  let wibble = wibble / 2
  wibble
}
",
        "wobble",
        find_position_of("wibble /"),
    );
}

#[test]
fn rename_shadowing_local_variable() {
    assert_rename!(
        "
pub fn main() {
  let wibble = 10
  let wibble = wibble / 2
  wibble
}
",
        "wobble",
        find_position_of("wibble").nth_occurrence(4),
    );
}

#[test]
fn rename_local_variable_record_access() {
    assert_rename!(
        "
type Wibble {
  Wibble(wibble: Int)
}

pub fn main() {
  let wibble = Wibble(wibble: 1)
  wibble.wibble
}
",
        "wobble",
        find_position_of("wibble."),
    );
}
#[test]
fn rename_local_variable_guard_clause() {
    assert_rename!(
        "
pub fn main() {
  let wibble = True
  case Nil {
    Nil if wibble -> todo
    _ -> panic
  }
  wibble || False
}
",
        "wobble",
        find_position_of("wibble ||"),
    );
}

#[test]
fn rename_local_variable_from_definition() {
    assert_rename!(
        "
pub fn main() {
  let wibble = 10
  let wobble = wibble + 1
  wobble - wibble
}
",
        "some_value",
        find_position_of("wibble =")
    );
}

#[test]
fn rename_local_variable_from_definition_nested_pattern() {
    assert_rename!(
        "
pub fn main() {
  let assert Ok([_, wibble, ..]) = Error(12)
  wibble
}
",
        "second_element",
        find_position_of("wibble,")
    );
}

#[test]
fn rename_local_variable_assignment_pattern() {
    assert_rename!(
        "
pub fn main() {
  let assert Error(12 as something) = Error(12)
  something
}
",
        "the_error",
        find_position_of("something").nth_occurrence(2)
    );
}

#[test]
fn rename_local_variable_from_definition_assignment_pattern() {
    assert_rename!(
        "
pub fn main() {
  let assert Error(12 as something) = Error(12)
  something
}
",
        "the_error",
        find_position_of("something)")
    );
}

#[test]
fn rename_local_variable_argument() {
    assert_rename!(
        "
pub fn add(first_number: Int, x: Int) -> Int {
  x + first_number
}
",
        "second_number",
        find_position_of("x +")
    );
}

#[test]
fn rename_local_variable_argument_from_definition() {
    assert_rename!(
        "
pub fn wibble(wibble: Float) {
  wibble /. 0.3
}
",
        "wobble",
        find_position_of("wibble:")
    );
}

#[test]
fn rename_local_variable_label_shorthand() {
    assert_rename!(
        "
type Wibble {
  Wibble(wibble: Int)
}

pub fn main() {
  let Wibble(wibble:) = todo
  wibble + 1
}
",
        "wobble",
        find_position_of("wibble +")
    );
}

#[test]
fn rename_local_variable_label_shorthand_from_definition() {
    assert_rename!(
        "
type Wibble {
  Wibble(wibble: Int)
}

pub fn main() {
  let Wibble(wibble:) = todo
  wibble + 1
}
",
        "wobble",
        find_position_of("wibble:)")
    );
}

#[test]
fn rename_local_variable_in_bit_array_pattern() {
    assert_rename!(
        "
pub fn starts_with(bits: BitArray, prefix: BitArray) -> Bool {
  let prefix_size = bit_size(prefix)

  case bits {
    <<pref:bits-size(prefix_size), _:bits>> if pref == prefix -> True
    _ -> False
  }
}
",
        "size_of_prefix",
        find_position_of("prefix_size =")
    );
}

#[test]
fn rename_local_variable_from_bit_array_pattern() {
    assert_rename!(
        "
pub fn starts_with(bits: BitArray, prefix: BitArray) -> Bool {
  let prefix_size = bit_size(prefix)

  case bits {
    <<pref:bits-size(prefix_size), _:bits>> if pref == prefix -> True
    _ -> False
  }
}
",
        "size_of_prefix",
        find_position_of("prefix_size)")
    );
}

#[test]
fn no_rename_keyword() {
    assert_no_rename!(
        "
pub fn main() {}
",
        "wibble",
        find_position_of("fn"),
    );
}

#[test]
fn no_rename_invalid_name() {
    assert_no_rename!(
        "
pub fn main() {
  let wibble = 10
  wibble
}
",
        "Not_AValid_Name",
        find_position_of("wibble").nth_occurrence(2)
    );
}

#[test]
fn rename_function_from_definition() {
    assert_rename!(
        (
            "mod",
            "
import app

fn wibble() {
  app.something()
}
"
        ),
        "
pub fn something() {
  something()
}

fn something_else() {
  something()
}
",
        "some_function",
        find_position_of("something")
    );
}

#[test]
fn rename_function_from_reference() {
    assert_rename!(
        (
            "mod",
            "
import app

fn wibble() {
  app.something()
}
"
        ),
        "
pub fn something() {
  something()
}

fn something_else() {
  something()
}
",
        "some_function",
        find_position_of("something").nth_occurrence(2)
    );
}

#[test]
fn rename_function_from_qualified_reference() {
    assert_rename!(
        (
            "mod",
            "
pub fn wibble() {
  wibble()
}
"
        ),
        "
import mod

pub fn main() {
  mod.wibble()
}
",
        "some_function",
        find_position_of("wibble")
    );
}

#[test]
fn rename_function_from_unqualified_reference() {
    assert_rename!(
        (
            "mod",
            "
pub fn wibble() {
  wibble()
}
"
        ),
        "
import mod.{wibble}

pub fn main() {
  wibble()
  mod.wibble()
}
",
        "some_function",
        find_position_of("wibble(")
    );
}

#[test]
fn rename_aliased_function() {
    assert_rename!(
        (
            "mod",
            "
import app.{something as something_else}

fn wibble() {
  something_else()
}
"
        ),
        "
pub fn something() {
  something()
}

fn something_else() {
  something()
}
",
        "some_function",
        find_position_of("something")
    );
}

#[test]
fn rename_function_shadowing_module() {
    let src = "
import gleam/list

pub fn list() {
  []
}

pub fn main() {
  list.map(todo, todo)
}
    ";

    assert_rename!(
        TestProject::for_source(src).add_hex_module("gleam/list", "pub fn map(_, _) {}"),
        "empty_list",
        find_position_of("list()")
    );
}

#[test]
fn rename_function_shadowed_by_field_access() {
    assert_rename!(
        (
            "mod",
            "
import app

type App {
  App(something: Int)
}

pub fn main() {
  let app = App(10)
  app.something
}
"
        ),
        "
pub fn something() {
  todo
}
",
        "function",
        find_position_of("something")
    );
}

#[test]
fn no_rename_function_with_invalid_name() {
    assert_no_rename!(
        "
pub fn main() {
  let wibble = 10
  wibble
}
",
        "Not_AValid_Name",
        find_position_of("main")
    );
}

#[test]
fn no_rename_function_from_other_package() {
    let src = "
import wibble

pub fn main() {
  wibble.wobble()
}
";

    assert_no_rename!(
        &TestProject::for_source(src).add_hex_module("wibble", "pub fn wobble() { todo }"),
        "something",
        find_position_of("wobble")
    );
}

#[test]
fn rename_constant_from_definition() {
    assert_rename!(
        (
            "mod",
            "
import app

fn wibble() {
  app.something
}
"
        ),
        "
pub const something = 10

pub fn main() {
  something + { 4 * something }
}
",
        "ten",
        find_position_of("something")
    );
}

#[test]
fn rename_constant_from_reference() {
    assert_rename!(
        (
            "mod",
            "
import app

fn wibble() {
  app.something
}
"
        ),
        "
pub const something = 10

pub fn main() {
  something + { 4 * something }
}
",
        "ten",
        find_position_of("something").nth_occurrence(2)
    );
}

#[test]
fn rename_constant_from_qualified_reference() {
    assert_rename!(
        (
            "mod",
            "
pub const something = 10

fn wibble() {
  something
}
"
        ),
        "
import mod

pub fn main() {
  mod.something
}
",
        "ten",
        find_position_of("something")
    );
}

#[test]
fn rename_constant_from_unqualified_reference() {
    assert_rename!(
        (
            "mod",
            "
pub const something = 10

fn wibble() {
  something
}
"
        ),
        "
import mod.{something}

pub fn main() {
  something + mod.something
}
",
        "ten",
        find_position_of("something +")
    );
}

#[test]
fn rename_aliased_constant() {
    assert_rename!(
        (
            "mod",
            "
import app.{something as some_constant}

fn wibble() {
  some_constant
}
"
        ),
        "
pub const something = 10

pub fn main() {
  something + { 4 * something }
}
",
        "ten",
        find_position_of("something")
    );
}

#[test]
fn rename_constant_shadowing_module() {
    let src = "
import gleam/list

const list = []

pub fn main() {
  list.map(todo, todo)
}
    ";

    assert_rename!(
        TestProject::for_source(src).add_hex_module("gleam/list", "pub fn map(_, _) {}"),
        "empty_list",
        find_position_of("list =")
    );
}

#[test]
fn rename_constant_shadowed_by_field_access() {
    assert_rename!(
        (
            "mod",
            "
import app

type App {
  App(something: Int)
}

pub fn main() {
  let app = App(10)
  app.something
}
"
        ),
        "
pub const something = 10
",
        "constant",
        find_position_of("something")
    );
}

#[test]
fn no_rename_constant_with_invalid_name() {
    assert_no_rename!(
        "
const value = 10
",
        "Ten",
        find_position_of("value")
    );
}

#[test]
fn no_rename_constant_from_other_package() {
    let src = "
import wibble

pub fn main() {
  wibble.wobble
}
";

    assert_no_rename!(
        &TestProject::for_source(src).add_hex_module("wibble", "pub const wobble = 2"),
        "something",
        find_position_of("wobble")
    );
}

#[test]
fn rename_type_variant_from_definition() {
    assert_rename!(
        (
            "mod",
            "
import app

fn wibble() {
  app.Constructor(4)
}
"
        ),
        "
pub type Wibble {
  Constructor(Int)
}

pub fn main() {
  Constructor(10)
}
",
        "Wibble",
        find_position_of("Constructor(Int")
    );
}

#[test]
fn rename_type_variant_from_reference() {
    assert_rename!(
        (
            "mod",
            "
import app

fn wibble() {
  app.Constructor(4)
}
"
        ),
        "
pub type Wibble {
  Constructor(Int)
}

pub fn main() {
  Constructor(10)
}
",
        "Wibble",
        find_position_of("Constructor(10")
    );
}

#[test]
fn rename_type_variant_from_qualified_reference() {
    assert_rename!(
        (
            "mod",
            "
pub type Wibble {
  Constructor(Int)
}

fn wibble() {
  Constructor(42)
}
"
        ),
        "
import mod

pub fn main() {
  mod.Constructor
}
",
        "Variant",
        find_position_of("Constructor")
    );
}

#[test]
fn rename_type_variant_from_unqualified_reference() {
    assert_rename!(
        (
            "mod",
            "
pub type Wibble {
  Constructor(Int)
}

fn wibble() {
  Constructor(81)
}
"
        ),
        "
import mod.{Constructor}

pub fn main() {
  #(Constructor(75), mod.Constructor(57))
}
",
        "Number",
        find_position_of("Constructor(75")
    );
}

#[test]
fn rename_aliased_type_variant() {
    assert_rename!(
        (
            "mod",
            "
import app.{Constructor as ValueConstructor}

fn wibble() {
  ValueConstructor(172)
}
"
        ),
        "
pub type Wibble {
  Constructor(Int)
}

pub fn main() {
  Constructor(42)
}
",
        "MakeAWibble",
        find_position_of("Constructor")
    );
}

#[test]
fn no_rename_type_variant_with_invalid_name() {
    assert_no_rename!(
        "
pub type Wibble {
  Constructor(Int)
}
",
        "name_in_snake_case",
        find_position_of("Constructor")
    );
}

#[test]
fn rename_custom_type_variant_pattern() {
    assert_rename!(
        "
pub type Type {
  X
  Y
}

pub fn main(t) {
  case t {
    X -> 0
    Y -> 0
  }
}
",
        "Renamed",
        find_position_of("X")
    );
}

#[test]
fn rename_imported_custom_type_variant_pattern() {
    assert_rename!(
        (
            "other",
            "
import app

pub fn main(t) {
  case t {
    app.X -> 0
    app.Y -> 0
  }
}
"
        ),
        "
pub type Type {
  X
  Y
}
",
        "Renamed",
        find_position_of("X")
    );
}

#[test]
fn rename_imported_unqualified_custom_type_variant_pattern() {
    assert_rename!(
        (
            "other",
            "
import app.{X, Y}

pub fn main(t) {
  case t {
    X -> 0
    Y -> 0
  }
}
"
        ),
        "
pub type Type {
  X
  Y
}
",
        "Renamed",
        find_position_of("X")
    );
}

#[test]
fn rename_type_variant_pattern_with_arguments() {
    assert_rename!(
        "
pub type Wibble {
  Wibble(Int)
  Wobble(Float)
}

fn wibble() {
  case Wibble(10) {
    Wibble(20) -> todo
    Wibble(_) -> panic
  }
}
",
        "Variant",
        find_position_of("Wibble(10)")
    );
}

#[test]
fn rename_type_variant_from_pattern() {
    assert_rename!(
        "
pub type Type {
  X
  Y
}

pub fn main(t) {
  case t {
    X -> 0
    Y -> 0
  }
}
",
        "Renamed",
        find_position_of("X ->")
    );
}

#[test]
fn no_rename_type_variant_from_other_package() {
    let src = "
import wibble

pub fn main() {
  wibble.Wibble(10)
}
";

    assert_no_rename!(
        &TestProject::for_source(src).add_hex_module("wibble", "pub type Wibble { Wibble(Int) }"),
        "Constructor",
        find_position_of("Wibble")
    );
}

#[test]
fn rename_value_in_nested_module() {
    assert_rename!(
        (
            "sub/mod",
            "
pub fn wibble() {
  wibble()
}
"
        ),
        "
import sub/mod

pub fn main() {
  mod.wibble()
}
",
        "some_function",
        find_position_of("wibble")
    );
}

#[test]
fn rename_value_in_aliased_module() {
    assert_rename!(
        (
            "mod",
            "
pub fn wibble() {
  wibble()
}
"
        ),
        "
import mod as the_module

pub fn main() {
  the_module.wibble()
}
",
        "some_function",
        find_position_of("wibble")
    );
}

#[test]
fn rename_aliased_value() {
    assert_rename!(
        (
            "mod",
            "
import app.{Wibble as Wobble}

fn wobble() {
  Wobble
}
"
        ),
        "
pub type Wibble { Wibble }

pub fn main() {
  Wibble
}
",
        "Wubble",
        find_position_of("Wibble }")
    );
}

#[test]
fn rename_type_from_definition() {
    assert_rename!(
        (
            "mod",
            "
import app

fn wibble() -> app.Wibble { todo }
"
        ),
        "
pub type Wibble { Constructor }

pub fn main(w: Wibble) -> Wibble { todo }
",
        "SomeType",
        find_position_of("Wibble")
    );
}

#[test]
fn rename_type_from_reference() {
    assert_rename!(
        (
            "mod",
            "
import app

fn wibble() -> app.Wibble { todo }
"
        ),
        "
pub type Wibble { Constructor }

pub fn main(w: Wibble) -> Wibble { todo }
",
        "SomeType",
        find_position_of("Wibble").nth_occurrence(2)
    );
}

#[test]
fn rename_type_from_qualified_reference() {
    assert_rename!(
        (
            "mod",
            "
pub type Wibble { Constructor }

fn wibble(w: Wibble) -> Wibble { todo }
"
        ),
        "
import mod

pub fn main(w: mod.Wibble) -> mod.Wibble { todo }
",
        "SomeType",
        find_position_of("Wibble")
    );
}

#[test]
fn rename_type_from_unqualified_reference() {
    assert_rename!(
        (
            "mod",
            "
pub type Wibble { Constructor }

fn wibble(w: Wibble) -> Wibble { todo }
"
        ),
        "
import mod.{type Wibble}

pub fn main(w: Wibble) -> mod.Wibble { todo }
",
        "SomeType",
        find_position_of("Wibble)")
    );
}

#[test]
fn rename_aliased_type() {
    assert_rename!(
        (
            "mod",
            "
import app.{type Wibble as Wobble}

fn wibble() -> Wobble { todo }
"
        ),
        "
pub type Wibble { Constructor }

pub fn main(w: Wibble) -> Wibble { todo }
",
        "SomeType",
        find_position_of("Wibble")
    );
}

#[test]
fn no_rename_type_with_invalid_name() {
    assert_no_rename!(
        "
type Wibble { Wobble }
",
        "a_type_name",
        find_position_of("Wibble")
    );
}

#[test]
fn no_rename_type_from_other_package() {
    let src = "
import wibble

pub fn main() -> wibble.Wibble { todo }
";

    assert_no_rename!(
        &TestProject::for_source(src).add_hex_module("wibble", "pub type Wibble { Wibble }"),
        "SomeType",
        find_position_of("Wibble")
    );
}

// https://github.com/gleam-lang/gleam/issues/4372
#[test]
fn rename_type_referenced_in_variant_constructor_argument() {
    assert_rename!(
        (
            "mod",
            "
import app

pub type Wobble {
  Wobble(w: app.Wibble)
}
"
        ),
        "
pub type Wibble {
  Wibble
}

pub fn main() {
  let wibble = Wibble
}
",
        "SomeType",
        find_position_of("Wibble")
    );
}

// https://github.com/gleam-lang/gleam/issues/4372
#[test]
fn rename_type_from_variant_constructor_argument() {
    assert_rename!(
        (
            "mod",
            "
pub type Wibble {
  Wibble
}

pub fn main() {
  let wibble = Wibble
}
"
        ),
        "
import mod

pub type Wobble {
  Wobble(w: mod.Wibble)
}
",
        "SomeType",
        find_position_of("Wibble")
    );
}

// https://github.com/gleam-lang/gleam/issues/4553
#[test]
fn rename_local_variable_with_label_shorthand() {
    assert_rename!(
        "
pub type Wibble {
  Wibble(first: Int, second: Int)
}

pub fn main() {
  let second = 2
  Wibble(first: 1, second:)
}
",
        "something",
        find_position_of("second =")
    );
}

// https://github.com/gleam-lang/gleam/issues/4748
#[test]
fn rename_alternative_pattern() {
    assert_rename!(
        "
pub fn main(x) {
  case x {
    #(wibble, [wobble]) | #(wobble, [wibble, _]) | #(_, [wibble, wobble, ..]) ->
      wibble + wobble
    _ -> 0
  }
}
",
        "new_name",
        find_position_of("wibble")
    );
}

#[test]
fn rename_alternative_pattern_from_usage() {
    assert_rename!(
        "
pub fn main(x) {
  case x {
    #(wibble, [wobble]) | #(wobble, [wibble, _]) | #(_, [wibble, wobble, ..]) ->
      wibble + wobble
    _ -> 0
  }
}
",
        "new_name",
        find_position_of("wibble +")
    );
}

// https://github.com/gleam-lang/gleam/issues/4605
#[test]
fn rename_prelude_value() {
    assert_rename!(
        "
pub fn main() {
  Ok(10)
}
",
        "Success",
        find_position_of("Ok")
    );
}
#[test]
fn rename_prelude_type() {
    assert_rename!(
        "
pub fn main() -> Result(Int, Nil) {
  Ok(10)
}
",
        "SuccessOrFailure",
        find_position_of("Result")
    );
}

#[test]
fn rename_variable_with_alternative_pattern_with_same_name() {
    assert_rename!(
        "
pub fn main(x) {
  let some_var = 10

  case x {
    #(some_var, []) | #(_, [some_var]) ->
      some_var
    _ -> 0
  }

  some_var
}
",
        "new_name",
        find_position_of("some_var")
    );
}

#[test]
fn rename_prelude_value_with_prelude_already_imported() {
    assert_rename!(
        "
import gleam

pub fn main() {
  Ok(gleam.Error(10))
}
",
        "Success",
        find_position_of("Ok")
    );
}

#[test]
fn rename_prelude_value_with_prelude_import_with_empty_braces() {
    assert_rename!(
        "
import gleam.{}

pub fn main() {
  Ok(gleam.Error(10))
}
",
        "Success",
        find_position_of("Ok")
    );
}

#[test]
fn rename_prelude_value_with_other_prelude_value_imported() {
    assert_rename!(
        "
import gleam.{Error}

pub fn main() {
  Ok(Error(10))
}
",
        "Success",
        find_position_of("Ok")
    );
}

#[test]
fn rename_prelude_type_with_prelude_value_imported_with_trailing_comma() {
    assert_rename!(
        "
import gleam.{Error,}

pub fn main() -> Result(Int, Nil) {
  Error(10)
}
",
        "OkOrError",
        find_position_of("Result")
    );
}

#[test]
fn rename_prelude_value_with_other_module_imported() {
    assert_rename!(
        ("something", "pub type Something"),
        "
import something

pub fn main() {
  Ok(10)
}
",
        "Success",
        find_position_of("Ok")
    );
}
