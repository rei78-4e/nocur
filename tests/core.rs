use nocur::{
    dsl::{eval, parse},
    editor::Editor,
    MAX_BUFFER_BYTES,
};

fn apply(source: &str, input: &str) -> String {
    eval(&parse(source).unwrap(), input).unwrap()
}

#[test]
fn primitives_and_newline_semantics() {
    assert_eq!(apply(r#"replace(/foo/, "bar")"#, "foo foo\n"), "bar bar\n");
    assert_eq!(
        apply(r#"replace("foo", "bar")"#, "foo food foo"),
        "bar bard bar"
    );
    assert_eq!(apply(r#"delete("foo")"#, "foo food"), " d");
    assert_eq!(
        apply(r#"filter("TODO")"#, "TODO x\nno\nTODO y"),
        "TODO x\nTODO y"
    );
    assert_eq!(
        apply(r#"replace(lines(2..2), "X\n")"#, "a\nb\nc\n"),
        "a\nX\nc\n"
    );
    assert_eq!(
        apply(r#"replace(lines(2..3), "X\n")"#, "a\nb\nc\nd\n"),
        "a\nX\nd\n"
    );
    assert_eq!(
        apply(r#"replace(2:2-2:3, "猫")"#, "a\n犬猫鳥\n"),
        "a\n犬猫\n"
    );
    assert_eq!(
        apply(r#"replace(1:1-1:1, "X")"#, "a\r\nb\r\n"),
        "X\r\nb\r\n"
    );
    assert!(eval(&parse(r#"replace(9:1-9:1, "x")"#).unwrap(), "a\n").is_err());
    assert!(eval(&parse(r#"replace(1:2-1:2, "x")"#).unwrap(), "a").is_err());
    assert!(parse(r#"replace(1:0-1:1, "x")"#).is_err());
    assert!(parse(r#"replace(0:1-0:1, "x")"#).is_err());
    assert!(eval(
        &parse(r#"replace(lines(2..9), "x")"#).unwrap(),
        "a\r\nb\r\n"
    )
    .is_err());
    assert_eq!(
        apply(r#"replace(lines(2..3), "X\r\n")"#, "a\r\nb\r\nc\r\nd\r\n"),
        "a\r\nX\r\nd\r\n"
    );
    assert_eq!(apply(r#"delete(/foo/)"#, "foo x foo"), " x ");
    assert_eq!(apply(r#"insert(2, "new\n")"#, "a\nb"), "a\nnew\nb");
    assert_eq!(apply(r#"insert(1, "x")"#, ""), "x");
    assert_eq!(apply("trim()", "  a\n "), "a");
    assert_eq!(apply("lines(2..3)", "a\nb\nc"), "b\nc");
    assert_eq!(apply("lines(1..10)", "a\n"), "a\n");
    assert_eq!(
        apply("filter(/TODO/)", "a\nTODO: x\nTODO: y"),
        "TODO: x\nTODO: y"
    );
    assert_eq!(apply("map(trim())", " a \n b \n"), "a\nb\n");
    assert_eq!(apply("map(trim())", ""), "");
    assert_eq!(apply("map(trim())", " a \r\n b "), "a\r\nb");
    assert_eq!(apply("filter(/^a$/)", "a\r\nb\r\n"), "a\r\n");
    assert_eq!(apply("", "a\r\nb\n"), "a\r\nb\n");
}

#[test]
fn nested_pipelines_regex_and_strings() {
    let source = r#"filter(/TODO/) |> map(replace(/TODO:/, "") |> trim())"#;
    assert_eq!(apply(source, "no\nTODO: あ \nTODO: b"), "あ\nb");
    assert_eq!(apply(r#"replace(/a\/b/, "x|>y,()")"#, "a/b"), "x|>y,()");
    assert_eq!(apply(r#"replace(/\s+/, " ")"#, "a\n b"), "a b");
    assert_eq!(apply(r#"replace(/a/, "$1")"#, "a"), "$1");
    assert_eq!(apply(r#"replace(/^/, "x")"#, "abc"), "xabc");
    assert_eq!(apply("map(map(trim()))", " a \n"), "a\n");
}

#[test]
fn invalid_and_effectful_expressions_are_rejected() {
    for source in [
        "replace(/[/, \"x\")",
        "trim() |> ",
        "lines(0..2)",
        "lines(3..1)",
        "clock()",
        "read(\"/etc/passwd\")",
        "random()",
        "map()",
        "trim() garbage",
        "trim(1)",
        r#"replace(/a/, "\q")"#,
        "replace(/abc, \"x\")",
    ] {
        assert!(parse(source).is_err(), "{source}");
    }
    assert!(eval(&parse("insert(3, \"x\")").unwrap(), "a").is_err());
    let nested = format!("{}trim(){}", "map(".repeat(18), ")".repeat(18));
    assert!(parse(&nested).is_err());
    assert!(parse(&" ".repeat(16 * 1024 + 1)).is_err());
}

#[test]
fn preview_is_derived_and_invalid_commit_is_atomic() {
    let mut editor = Editor::new("foo".into());
    editor.set_expression(r#"replace(/foo/, "bar")"#.into());
    assert_eq!(editor.state.preview().unwrap(), "bar");
    assert_eq!(editor.state.preview().unwrap(), "bar");
    assert_eq!(editor.state.committed(), "foo");
    editor.set_expression("replace(".into());
    assert!(editor.commit().is_err());
    assert_eq!(editor.position(), (0, 0));
    assert_eq!(editor.state.committed(), "foo");
    assert!(editor.state.preview().is_err());
}

#[test]
fn history_replay_export_and_branching() {
    let mut editor = Editor::new("foo\n".into());
    editor.set_expression(r#"replace(/foo/, "bar")"#.into());
    editor.commit().unwrap();
    assert_eq!(editor.draft(), "");
    editor.set_expression("trim()".into());
    editor.commit().unwrap();
    assert_eq!(editor.state.committed(), "bar");
    assert_eq!(editor.replay().unwrap(), "bar");
    assert_eq!(apply(&editor.export_script(), "foo\n"), "bar");
    assert!(editor.undo());
    assert_eq!(editor.state.committed(), "bar\n");
    assert!(editor.redo());
    assert_eq!(editor.state.committed(), "bar");
    assert!(editor.undo());
    editor.set_expression(r#"replace(/bar/, "other")"#.into());
    editor.commit().unwrap();
    assert!(!editor.redo());
    assert_eq!(editor.replay().unwrap(), editor.state.committed());
    assert_eq!(apply(&editor.export_script(), "foo\n"), "other\n");
    assert!(editor.undo());
    assert!(editor.undo());
    assert!(!editor.undo());
    assert_eq!(editor.state.committed(), "foo\n");
    assert_eq!(editor.export_script(), "");
}

#[test]
fn size_limits_and_repeatability() {
    let input = "a".repeat(MAX_BUFFER_BYTES / 2 + 1);
    assert!(eval(&parse(r#"replace(/a/, "xx")"#).unwrap(), &input).is_err());
    let source = parse(r#"map(trim() |> replace(/\s+/, " "))"#).unwrap();
    let input = "  あ  b \n";
    assert_eq!(eval(&source, input), eval(&source, input));
    assert_eq!(input, "  あ  b \n");
}
