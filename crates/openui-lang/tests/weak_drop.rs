//! Freeing a tree whose `Arc`s have weak references still needs no stack
//! (#242). `Arc::get_mut` declines while a weak reference exists, even for the
//! last strong owner; the freeing loop then has to take the payload out some
//! other way, or Rust drops the child recursively, level after level.
//!
//! Every test runs on a thread with a 64 KiB stack, and the file is its own
//! test binary because a stack overflow aborts the process. Run it unoptimised
//! too (`scripts/check.sh` lists it in the stack step). The tests share the
//! allocator's live-byte count, so they take turns.

use std::sync::{Arc, Mutex, MutexGuard, Weak};

use openui_lang::{Ast, Element, EvaluationSession, JsObject, Limits, Value, parse_json_value};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(1024 * 1024 * 1024);

static TURN: Mutex<()> = Mutex::new(());

fn take_turn() -> MutexGuard<'static, ()> {
    TURN.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Run `f` on a 64 KiB stack.
fn small<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

/// Weak references to every `Arc` a chain is built from.
#[derive(Default)]
struct Watch(Vec<Box<dyn Fn() -> bool + Send>>);

impl Watch {
    fn add<T: Send + Sync + 'static>(&mut self, arc: &Arc<T>) {
        let weak: Weak<T> = Arc::downgrade(arc);
        self.0.push(Box::new(move || weak.upgrade().is_some()));
    }

    fn alive(&self) -> usize {
        self.0.iter().filter(|alive| alive()).count()
    }
}

fn watched<T: Send + Sync + 'static>(watch: &mut Watch, value: T) -> Arc<T> {
    let arc = Arc::new(value);
    watch.add(&arc);
    arc
}

fn element(props: JsObject<Value>) -> Element {
    Element {
        type_name: "Box".into(),
        props,
        partial: false,
        has_dynamic_props: false,
        statement_id: None,
    }
}

/// A chain of runtime expressions, each holding the next.
fn value_ast_chain(depth: usize, watch: &mut Watch) -> Value {
    let ast = ast_chain(Shape::Unary, depth, watch);
    Value::Ast(watched(watch, ast))
}

#[derive(Clone, Copy, Debug)]
enum Shape {
    Unary,
    BinaryLeft,
    BinaryRight,
    TernaryCondition,
    TernaryThen,
    TernaryOtherwise,
    Member,
    IndexObject,
    IndexKey,
    Assign,
    Array,
    Object,
    CallArgs,
    CallMappedProps,
}

const SHAPES: [Shape; 14] = [
    Shape::Unary,
    Shape::BinaryLeft,
    Shape::BinaryRight,
    Shape::TernaryCondition,
    Shape::TernaryThen,
    Shape::TernaryOtherwise,
    Shape::Member,
    Shape::IndexObject,
    Shape::IndexKey,
    Shape::Assign,
    Shape::Array,
    Shape::Object,
    Shape::CallArgs,
    Shape::CallMappedProps,
];

/// An expression `depth` levels deep along one kind of edge, a weak
/// reference on every `Arc`.
fn ast_chain(shape: Shape, depth: usize, watch: &mut Watch) -> Ast {
    let mut ast = Ast::Null;
    for _ in 0..depth {
        ast = match shape {
            Shape::Unary => Ast::UnaryOp {
                op: "!",
                operand: watched(watch, ast),
            },
            Shape::BinaryLeft => Ast::BinOp {
                op: "+",
                left: watched(watch, ast),
                right: watched(watch, Ast::Null),
            },
            Shape::BinaryRight => Ast::BinOp {
                op: "+",
                left: watched(watch, Ast::Null),
                right: watched(watch, ast),
            },
            Shape::TernaryCondition => Ast::Ternary {
                cond: watched(watch, ast),
                then: watched(watch, Ast::Null),
                otherwise: watched(watch, Ast::Null),
            },
            Shape::TernaryThen => Ast::Ternary {
                cond: watched(watch, Ast::Null),
                then: watched(watch, ast),
                otherwise: watched(watch, Ast::Null),
            },
            Shape::TernaryOtherwise => Ast::Ternary {
                cond: watched(watch, Ast::Null),
                then: watched(watch, Ast::Null),
                otherwise: watched(watch, ast),
            },
            Shape::Member => Ast::Member {
                obj: watched(watch, ast),
                field: "f".into(),
            },
            Shape::IndexObject => Ast::Index {
                obj: watched(watch, ast),
                index: watched(watch, Ast::Null),
            },
            Shape::IndexKey => Ast::Index {
                obj: watched(watch, Ast::Null),
                index: watched(watch, ast),
            },
            Shape::Assign => Ast::Assign {
                target: "$x".into(),
                value: watched(watch, ast),
            },
            Shape::Array => Ast::Arr(watched(watch, vec![ast])),
            Shape::Object => Ast::Obj(watched(watch, vec![("k".to_string(), ast)])),
            Shape::CallArgs => Ast::Comp {
                name: "C".into(),
                args: watched(watch, vec![ast]),
                function: false,
                mapped_props: None,
            },
            Shape::CallMappedProps => {
                let mut props = JsObject::new();
                props.set("p", ast);
                Ast::Comp {
                    name: "C".into(),
                    args: watched(watch, Vec::new()),
                    function: false,
                    mapped_props: Some(watched(watch, props)),
                }
            }
        };
    }
    ast
}

/// How deep a chain of single-child arrays, objects and elements goes, and
/// what it ends in; an iterative walk, so it needs no stack either.
fn chain_depth(value: &Value) -> (usize, bool) {
    let mut depth = 0;
    let mut here = value;
    loop {
        here = match here {
            Value::Arr(items) => &items[0],
            Value::Obj(object) => object.get("k").unwrap(),
            Value::Element(element) => element.props.get("child").unwrap(),
            Value::Num(n) => return (depth, *n == 0.0),
            _ => return (depth, false),
        };
        depth += 1;
    }
}

/// The reviewer's reproduction: a JSON value the evaluator refuses leaves a
/// weak reference at every level.
fn refused_json_value(open: &str, close: &str) {
    let depth = 4096;
    let json = format!("{}0{}", open.repeat(depth), close.repeat(depth));
    small(move || {
        let value = parse_json_value(&json, depth + 1).unwrap();
        let limits = Limits::DEFAULT.with_depth(depth + 1).with_result_size(16);
        let session = EvaluationSession::with_limits(&(), json.len(), limits);
        assert!(!session.with_evaluator(|eval| eval.reserve_value(&value)));
        drop(value);
        drop(session);
    });
}

#[test]
fn a_refused_json_array_chain_frees_on_a_small_stack() {
    let _turn = take_turn();
    refused_json_value("[", "]");
}

#[test]
fn a_refused_json_object_chain_frees_on_a_small_stack() {
    let _turn = take_turn();
    refused_json_value("{\"k\":", "}");
}

#[test]
fn a_directly_built_array_chain_with_weak_observers_frees_on_a_small_stack() {
    let _turn = take_turn();
    small(|| {
        let mut watch = Watch::default();
        let value = chain(Kind::Array, 100_000, &mut watch);
        assert_eq!(watch.alive(), 100_000);
        drop(value);
        assert_eq!(watch.alive(), 0);
    });
}

#[test]
fn object_element_and_runtime_expression_chains_free_on_a_small_stack() {
    let _turn = take_turn();
    small(|| {
        for kind in [Kind::Object, Kind::Element] {
            let mut watch = Watch::default();
            let value = chain(kind, 100_000, &mut watch);
            assert_eq!(watch.alive(), 100_000, "{kind:?}");
            drop(value);
            assert_eq!(watch.alive(), 0, "{kind:?}");
        }
        let mut watch = Watch::default();
        let value = value_ast_chain(100_000, &mut watch);
        assert_eq!(watch.alive(), 100_001);
        drop(value);
        assert_eq!(watch.alive(), 0);
    });
}

#[test]
fn every_expression_edge_frees_on_a_small_stack() {
    let _turn = take_turn();
    small(|| {
        for shape in SHAPES {
            for depth in [4096, 100_000] {
                let mut watch = Watch::default();
                let ast = ast_chain(shape, depth, &mut watch);
                assert!(watch.alive() >= depth, "{shape:?}");
                drop(ast);
                assert_eq!(watch.alive(), 0, "{shape:?} at {depth}");
            }
        }
    });
}

/// One more level of the same kind as `build` makes.
fn wrap(kind: Kind, value: Value, watch: &mut Watch) -> Value {
    match kind {
        Kind::Array => Value::Arr(watched(watch, vec![value])),
        Kind::Object => {
            let mut object = JsObject::new();
            object.set("k", value);
            Value::Obj(watched(watch, object))
        }
        Kind::Element => {
            let mut props = JsObject::new();
            props.set("child", value);
            Value::Element(watched(watch, element(props)))
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Array,
    Object,
    Element,
}

const KINDS: [Kind; 3] = [Kind::Array, Kind::Object, Kind::Element];

fn chain(kind: Kind, depth: usize, watch: &mut Watch) -> Value {
    let mut value = Value::Num(0.0);
    for _ in 0..depth {
        value = wrap(kind, value, watch);
    }
    value
}

/// A deep chain with two strong owners of one level: dropping one owner frees
/// the levels above it and leaves the shared levels whole for the other.
#[test]
fn a_shared_subtree_survives_its_other_owners_drop_and_frees_with_it() {
    let _turn = take_turn();
    small(|| {
        for kind in KINDS {
            let (above, below) = (60_000, 40_000);
            let mut lower = Watch::default();
            let mut upper = Watch::default();
            let mut value = chain(kind, below, &mut lower);
            let kept = value.clone();
            for _ in 0..above {
                value = wrap(kind, value, &mut upper);
            }
            assert_eq!(chain_depth(&value), (above + below, true), "{kind:?}");
            assert_eq!(upper.alive(), above, "{kind:?}");
            assert_eq!(lower.alive(), below, "{kind:?}");

            drop(value);
            assert_eq!(upper.alive(), 0, "{kind:?}: the levels above are freed");
            assert_eq!(lower.alive(), below, "{kind:?}: the shared levels stay");
            assert_eq!(chain_depth(&kept), (below, true), "{kind:?}");

            drop(kept);
            assert_eq!(lower.alive(), 0, "{kind:?}: the last owner frees them");
        }
    });
}

/// A weak reference that upgrades is a second strong owner, before and after
/// the chain's own drop.
#[test]
fn an_upgraded_weak_reference_keeps_its_levels_whole_across_the_drop() {
    let _turn = take_turn();
    small(|| {
        let depth = 100_000;
        let middle = depth / 2;
        let mut watch = Watch::default();
        let mut observer = None;
        let mut value = Value::Num(0.0);
        for level in 0..depth {
            let arc = watched(&mut watch, vec![value]);
            if level == middle {
                observer = Some(Arc::downgrade(&arc));
            }
            value = Value::Arr(arc);
        }
        let observer = observer.unwrap();

        let before = Value::Arr(observer.upgrade().expect("alive before the drop"));
        drop(value);
        let after = Value::Arr(observer.upgrade().expect("still alive: an owner remains"));
        let levels = middle + 1;
        assert_eq!(watch.alive(), levels);
        assert_eq!(chain_depth(&before), (levels, true));
        assert_eq!(chain_depth(&after), (levels, true));

        drop(before);
        assert!(observer.upgrade().is_some());
        assert_eq!(chain_depth(&after), (levels, true));
        drop(after);
        assert!(observer.upgrade().is_none());
        assert_eq!(watch.alive(), 0);
    });
}

/// Every weak reference fails once the last owner is gone, and the memory
/// comes back (the weak references' own blocks once they are dropped too).
///
/// The test harness allocates on its own threads while this runs (it starts
/// the next test), so an attempt that ends off baseline is repeated: a leak
/// is the same on every attempt, a harness allocation is not.
#[test]
fn freeing_upgrades_no_weak_reference_and_leaves_no_live_bytes() {
    let _turn = take_turn();
    small(|| ()); // anything a first thread leaves behind
    let mut offsets = Vec::new();
    for _ in 0..20 {
        let before = test_alloc::live_bytes();
        small(|| {
            for kind in KINDS {
                let mut watch = Watch::default();
                drop(chain(kind, 30_000, &mut watch));
                assert_eq!(watch.alive(), 0, "{kind:?}");
            }
            for shape in SHAPES {
                let mut watch = Watch::default();
                drop(ast_chain(shape, 30_000, &mut watch));
                assert_eq!(watch.alive(), 0, "{shape:?}");
            }
            let mut watch = Watch::default();
            drop(value_ast_chain(30_000, &mut watch));
            assert_eq!(watch.alive(), 0);
        });
        let after = test_alloc::live_bytes();
        if after == before {
            return;
        }
        offsets.push(after as i64 - before as i64);
    }
    panic!("live bytes never returned to the baseline; offsets: {offsets:?}");
}

/// #234's bound: a wide collection frees in auxiliary memory that doesn't
/// grow with its width, though every child, and the collection itself, has a
/// weak observer, and the last child is a deep chain.
#[test]
fn a_wide_collection_of_watched_children_frees_in_bounded_memory() {
    const AUXILIARY_BOUND: usize = 16 * 1024;
    let _turn = take_turn();
    small(|| {
        for watched_container in [false, true] {
            for object in [false, true] {
                let mut watch = Watch::default();
                let width = 65_536;
                let wide = if object {
                    let mut props = JsObject::with_capacity(width);
                    for key in 0..width {
                        props.define(key.to_string(), watched_child(key, width, &mut watch));
                    }
                    Value::Obj(if watched_container {
                        watched(&mut watch, props)
                    } else {
                        Arc::new(props)
                    })
                } else {
                    let items: Vec<Value> = (0..width)
                        .map(|key| watched_child(key, width, &mut watch))
                        .collect();
                    Value::Arr(if watched_container {
                        watched(&mut watch, items)
                    } else {
                        Arc::new(items)
                    })
                };
                assert!(watch.alive() >= width);

                test_alloc::reset_peak();
                let input_live = test_alloc::peak();
                let before = test_alloc::allocated_bytes();
                drop(wide);
                let allocated = test_alloc::allocated_bytes() - before;
                let peak = test_alloc::peak() - input_live;
                test_alloc::reset_peak();

                let shape = if object { "object" } else { "array" };
                eprintln!(
                    "{shape}, container watched {watched_container}: {peak} bytes peak above input, {allocated} bytes allocated"
                );
                assert_eq!(watch.alive(), 0);
                assert!(
                    peak <= AUXILIARY_BOUND && allocated <= AUXILIARY_BOUND,
                    "{shape}, container watched {watched_container}: {peak} bytes peak, {allocated} allocated"
                );
            }
        }
    });
}

/// A watched array holding a leaf; the last child is a 50,000-level watched
/// chain instead, so the wide collection is deep as well.
fn watched_child(key: usize, width: usize, watch: &mut Watch) -> Value {
    if key + 1 == width {
        chain(Kind::Array, 50_000, watch)
    } else {
        Value::Arr(watched(watch, vec![Value::Num(key as f64)]))
    }
}
