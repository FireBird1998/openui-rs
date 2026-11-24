//! Captured from the derived `Debug` impls, before #231 replaced them.

/// (case, what, form, text)
pub const GOLDEN: &[(&str, &str, &str, &str)] = &[
    (
        "wide",
        "type",
        "compact",
        r####"Default(Union([ArrayWithMinimum(Number, Number(2)), Object([FieldDefinition { name: "f", ty: Default(String, DefaultValue(Object {"b": Array [Bool(true), Null, Number(-1.5), String("s\"q")], "a": Object {}, "c": Array []})), optional: true }]), Enum(["x", "y\n"]), Component("Other"), Action(Some("OpenUrl")), Action(None), Record(Boolean), Nullable(Any), Named("Label", Unknown), Object([]), Union([])]), DefaultValue(Object {"x": Array [Number(1), Object {"y": Null}]}))"####,
    ),
    (
        "wide",
        "type",
        "pretty",
        r####"Default(
    Union(
        [
            ArrayWithMinimum(
                Number,
                Number(2),
            ),
            Object(
                [
                    FieldDefinition {
                        name: "f",
                        ty: Default(
                            String,
                            DefaultValue(
                                Object {
                                    "b": Array [
                                        Bool(true),
                                        Null,
                                        Number(-1.5),
                                        String("s\"q"),
                                    ],
                                    "a": Object {},
                                    "c": Array [],
                                },
                            ),
                        ),
                        optional: true,
                    },
                ],
            ),
            Enum(
                [
                    "x",
                    "y\n",
                ],
            ),
            Component(
                "Other",
            ),
            Action(
                Some(
                    "OpenUrl",
                ),
            ),
            Action(
                None,
            ),
            Record(
                Boolean,
            ),
            Nullable(
                Any,
            ),
            Named(
                "Label",
                Unknown,
            ),
            Object(
                [],
            ),
            Union(
                [],
            ),
        ],
    ),
    DefaultValue(
        Object {
            "x": Array [
                Number(1),
                Object {
                    "y": Null,
                },
            ],
        },
    ),
)"####,
    ),
    (
        "wide",
        "definition",
        "compact",
        r####"LibraryDefinition { components: [ComponentDefinition { name: "Deep", description: "", props: [PropDefinition { name: "v", ty: Default(Union([ArrayWithMinimum(Number, Number(2)), Object([FieldDefinition { name: "f", ty: Default(String, DefaultValue(Object {"b": Array [Bool(true), Null, Number(-1.5), String("s\"q")], "a": Object {}, "c": Array []})), optional: true }]), Enum(["x", "y\n"]), Component("Other"), Action(Some("OpenUrl")), Action(None), Record(Boolean), Nullable(Any), Named("Label", Unknown), Object([]), Union([])]), DefaultValue(Object {"x": Array [Number(1), Object {"y": Null}]})), optional: false, binding: false }] }, ComponentDefinition { name: "Other", description: "", props: [] }], component_groups: Some([ComponentGroup { name: "G", components: ["Deep", "Other"], notes: Some(["n1", "n2"]) }, ComponentGroup { name: "H", components: [], notes: None }]), functions: [FunctionDefinition { name: "F", description: "d", params: [FieldDefinition { name: "p", ty: Array(String), optional: true }], returns: Some(Nullable(Number)) }], actions: [ActionDefinition { name: "Go", description: "g", params: [FieldDefinition { name: "q", ty: Object([FieldDefinition { name: "r", ty: Boolean, optional: false }]), optional: false }] }], root: Some("Deep"), id: Some("lib") }"####,
    ),
    (
        "wide",
        "definition",
        "pretty",
        r####"LibraryDefinition {
    components: [
        ComponentDefinition {
            name: "Deep",
            description: "",
            props: [
                PropDefinition {
                    name: "v",
                    ty: Default(
                        Union(
                            [
                                ArrayWithMinimum(
                                    Number,
                                    Number(2),
                                ),
                                Object(
                                    [
                                        FieldDefinition {
                                            name: "f",
                                            ty: Default(
                                                String,
                                                DefaultValue(
                                                    Object {
                                                        "b": Array [
                                                            Bool(true),
                                                            Null,
                                                            Number(-1.5),
                                                            String("s\"q"),
                                                        ],
                                                        "a": Object {},
                                                        "c": Array [],
                                                    },
                                                ),
                                            ),
                                            optional: true,
                                        },
                                    ],
                                ),
                                Enum(
                                    [
                                        "x",
                                        "y\n",
                                    ],
                                ),
                                Component(
                                    "Other",
                                ),
                                Action(
                                    Some(
                                        "OpenUrl",
                                    ),
                                ),
                                Action(
                                    None,
                                ),
                                Record(
                                    Boolean,
                                ),
                                Nullable(
                                    Any,
                                ),
                                Named(
                                    "Label",
                                    Unknown,
                                ),
                                Object(
                                    [],
                                ),
                                Union(
                                    [],
                                ),
                            ],
                        ),
                        DefaultValue(
                            Object {
                                "x": Array [
                                    Number(1),
                                    Object {
                                        "y": Null,
                                    },
                                ],
                            },
                        ),
                    ),
                    optional: false,
                    binding: false,
                },
            ],
        },
        ComponentDefinition {
            name: "Other",
            description: "",
            props: [],
        },
    ],
    component_groups: Some(
        [
            ComponentGroup {
                name: "G",
                components: [
                    "Deep",
                    "Other",
                ],
                notes: Some(
                    [
                        "n1",
                        "n2",
                    ],
                ),
            },
            ComponentGroup {
                name: "H",
                components: [],
                notes: None,
            },
        ],
    ),
    functions: [
        FunctionDefinition {
            name: "F",
            description: "d",
            params: [
                FieldDefinition {
                    name: "p",
                    ty: Array(
                        String,
                    ),
                    optional: true,
                },
            ],
            returns: Some(
                Nullable(
                    Number,
                ),
            ),
        },
    ],
    actions: [
        ActionDefinition {
            name: "Go",
            description: "g",
            params: [
                FieldDefinition {
                    name: "q",
                    ty: Object(
                        [
                            FieldDefinition {
                                name: "r",
                                ty: Boolean,
                                optional: false,
                            },
                        ],
                    ),
                    optional: false,
                },
            ],
        },
    ],
    root: Some(
        "Deep",
    ),
    id: Some(
        "lib",
    ),
}"####,
    ),
    (
        "wide",
        "library",
        "compact",
        r####"DefinedLibrary { definition: LibraryDefinition { components: [ComponentDefinition { name: "Deep", description: "", props: [PropDefinition { name: "v", ty: Default(Union([ArrayWithMinimum(Number, Number(2)), Object([FieldDefinition { name: "f", ty: Default(String, DefaultValue(Object {"b": Array [Bool(true), Null, Number(-1.5), String("s\"q")], "a": Object {}, "c": Array []})), optional: true }]), Enum(["x", "y\n"]), Component("Other"), Action(Some("OpenUrl")), Action(None), Record(Boolean), Nullable(Any), Named("Label", Unknown), Object([]), Union([])]), DefaultValue(Object {"x": Array [Number(1), Object {"y": Null}]})), optional: false, binding: false }] }, ComponentDefinition { name: "Other", description: "", props: [] }], component_groups: Some([ComponentGroup { name: "G", components: ["Deep", "Other"], notes: Some(["n1", "n2"]) }, ComponentGroup { name: "H", components: [], notes: None }]), functions: [FunctionDefinition { name: "F", description: "d", params: [FieldDefinition { name: "p", ty: Array(String), optional: true }], returns: Some(Nullable(Number)) }], actions: [ActionDefinition { name: "Go", description: "g", params: [FieldDefinition { name: "q", ty: Object([FieldDefinition { name: "r", ty: Boolean, optional: false }]), optional: false }] }], root: Some("Deep"), id: Some("lib") }, components: [0, 1] }"####,
    ),
    (
        "wide",
        "library",
        "pretty",
        r####"DefinedLibrary {
    definition: LibraryDefinition {
        components: [
            ComponentDefinition {
                name: "Deep",
                description: "",
                props: [
                    PropDefinition {
                        name: "v",
                        ty: Default(
                            Union(
                                [
                                    ArrayWithMinimum(
                                        Number,
                                        Number(2),
                                    ),
                                    Object(
                                        [
                                            FieldDefinition {
                                                name: "f",
                                                ty: Default(
                                                    String,
                                                    DefaultValue(
                                                        Object {
                                                            "b": Array [
                                                                Bool(true),
                                                                Null,
                                                                Number(-1.5),
                                                                String("s\"q"),
                                                            ],
                                                            "a": Object {},
                                                            "c": Array [],
                                                        },
                                                    ),
                                                ),
                                                optional: true,
                                            },
                                        ],
                                    ),
                                    Enum(
                                        [
                                            "x",
                                            "y\n",
                                        ],
                                    ),
                                    Component(
                                        "Other",
                                    ),
                                    Action(
                                        Some(
                                            "OpenUrl",
                                        ),
                                    ),
                                    Action(
                                        None,
                                    ),
                                    Record(
                                        Boolean,
                                    ),
                                    Nullable(
                                        Any,
                                    ),
                                    Named(
                                        "Label",
                                        Unknown,
                                    ),
                                    Object(
                                        [],
                                    ),
                                    Union(
                                        [],
                                    ),
                                ],
                            ),
                            DefaultValue(
                                Object {
                                    "x": Array [
                                        Number(1),
                                        Object {
                                            "y": Null,
                                        },
                                    ],
                                },
                            ),
                        ),
                        optional: false,
                        binding: false,
                    },
                ],
            },
            ComponentDefinition {
                name: "Other",
                description: "",
                props: [],
            },
        ],
        component_groups: Some(
            [
                ComponentGroup {
                    name: "G",
                    components: [
                        "Deep",
                        "Other",
                    ],
                    notes: Some(
                        [
                            "n1",
                            "n2",
                        ],
                    ),
                },
                ComponentGroup {
                    name: "H",
                    components: [],
                    notes: None,
                },
            ],
        ),
        functions: [
            FunctionDefinition {
                name: "F",
                description: "d",
                params: [
                    FieldDefinition {
                        name: "p",
                        ty: Array(
                            String,
                        ),
                        optional: true,
                    },
                ],
                returns: Some(
                    Nullable(
                        Number,
                    ),
                ),
            },
        ],
        actions: [
            ActionDefinition {
                name: "Go",
                description: "g",
                params: [
                    FieldDefinition {
                        name: "q",
                        ty: Object(
                            [
                                FieldDefinition {
                                    name: "r",
                                    ty: Boolean,
                                    optional: false,
                                },
                            ],
                        ),
                        optional: false,
                    },
                ],
            },
        ],
        root: Some(
            "Deep",
        ),
        id: Some(
            "lib",
        ),
    },
    components: [
        0,
        1,
    ],
}"####,
    ),
    (
        "object-chain-20",
        "type",
        "compact",
        r####"Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: String, optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }])"####,
    ),
    (
        "object-chain-20",
        "type",
        "pretty",
        r####"Object(
    [
        FieldDefinition {
            name: "f",
            ty: Object(
                [
                    FieldDefinition {
                        name: "f",
                        ty: Object(
                            [
                                FieldDefinition {
                                    name: "f",
                                    ty: Object(
                                        [
                                            FieldDefinition {
                                                name: "f",
                                                ty: Object(
                                                    [
                                                        FieldDefinition {
                                                            name: "f",
                                                            ty: Object(
                                                                [
                                                                    FieldDefinition {
                                                                        name: "f",
                                                                        ty: Object(
                                                                            [
                                                                                FieldDefinition {
                                                                                    name: "f",
                                                                                    ty: Object(
                                                                                        [
                                                                                            FieldDefinition {
                                                                                                name: "f",
                                                                                                ty: Object(
                                                                                                    [
                                                                                                        FieldDefinition {
                                                                                                            name: "f",
                                                                                                            ty: Object(
                                                                                                                [
                                                                                                                    FieldDefinition {
                                                                                                                        name: "f",
                                                                                                                        ty: Object(
                                                                                                                            [
                                                                                                                                FieldDefinition {
                                                                                                                                    name: "f",
                                                                                                                                    ty: Object(
                                                                                                                                        [
                                                                                                                                            FieldDefinition {
                                                                                                                                                name: "f",
                                                                                                                                                ty: Object(
                                                                                                                                                    [
                                                                                                                                                        FieldDefinition {
                                                                                                                                                            name: "f",
                                                                                                                                                            ty: Object(
                                                                                                                                                                [
                                                                                                                                                                    FieldDefinition {
                                                                                                                                                                        name: "f",
                                                                                                                                                                        ty: Object(
                                                                                                                                                                            [
                                                                                                                                                                                FieldDefinition {
                                                                                                                                                                                    name: "f",
                                                                                                                                                                                    ty: Object(
                                                                                                                                                                                        [
                                                                                                                                                                                            FieldDefinition {
                                                                                                                                                                                                name: "f",
                                                                                                                                                                                                ty: Object(
                                                                                                                                                                                                    [
                                                                                                                                                                                                        FieldDefinition {
                                                                                                                                                                                                            name: "f",
                                                                                                                                                                                                            ty: Object(
                                                                                                                                                                                                                [
                                                                                                                                                                                                                    FieldDefinition {
                                                                                                                                                                                                                        name: "f",
                                                                                                                                                                                                                        ty: Object(
                                                                                                                                                                                                                            [
                                                                                                                                                                                                                                FieldDefinition {
                                                                                                                                                                                                                                    name: "f",
                                                                                                                                                                                                                                    ty: String,
                                                                                                                                                                                                                                    optional: true,
                                                                                                                                                                                                                                },
                                                                                                                                                                                                                            ],
                                                                                                                                                                                                                        ),
                                                                                                                                                                                                                        optional: true,
                                                                                                                                                                                                                    },
                                                                                                                                                                                                                ],
                                                                                                                                                                                                            ),
                                                                                                                                                                                                            optional: true,
                                                                                                                                                                                                        },
                                                                                                                                                                                                    ],
                                                                                                                                                                                                ),
                                                                                                                                                                                                optional: true,
                                                                                                                                                                                            },
                                                                                                                                                                                        ],
                                                                                                                                                                                    ),
                                                                                                                                                                                    optional: true,
                                                                                                                                                                                },
                                                                                                                                                                            ],
                                                                                                                                                                        ),
                                                                                                                                                                        optional: true,
                                                                                                                                                                    },
                                                                                                                                                                ],
                                                                                                                                                            ),
                                                                                                                                                            optional: true,
                                                                                                                                                        },
                                                                                                                                                    ],
                                                                                                                                                ),
                                                                                                                                                optional: true,
                                                                                                                                            },
                                                                                                                                        ],
                                                                                                                                    ),
                                                                                                                                    optional: true,
                                                                                                                                },
                                                                                                                            ],
                                                                                                                        ),
                                                                                                                        optional: true,
                                                                                                                    },
                                                                                                                ],
                                                                                                            ),
                                                                                                            optional: true,
                                                                                                        },
                                                                                                    ],
                                                                                                ),
                                                                                                optional: true,
                                                                                            },
                                                                                        ],
                                                                                    ),
                                                                                    optional: true,
                                                                                },
                                                                            ],
                                                                        ),
                                                                        optional: true,
                                                                    },
                                                                ],
                                                            ),
                                                            optional: true,
                                                        },
                                                    ],
                                                ),
                                                optional: true,
                                            },
                                        ],
                                    ),
                                    optional: true,
                                },
                            ],
                        ),
                        optional: true,
                    },
                ],
            ),
            optional: true,
        },
    ],
)"####,
    ),
    (
        "object-chain-20",
        "definition",
        "compact",
        r####"LibraryDefinition { components: [ComponentDefinition { name: "Deep", description: "", props: [PropDefinition { name: "v", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: String, optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: false, binding: false }] }, ComponentDefinition { name: "Other", description: "", props: [] }], component_groups: None, functions: [], actions: [], root: None, id: None }"####,
    ),
    (
        "object-chain-20",
        "definition",
        "pretty",
        r####"LibraryDefinition {
    components: [
        ComponentDefinition {
            name: "Deep",
            description: "",
            props: [
                PropDefinition {
                    name: "v",
                    ty: Object(
                        [
                            FieldDefinition {
                                name: "f",
                                ty: Object(
                                    [
                                        FieldDefinition {
                                            name: "f",
                                            ty: Object(
                                                [
                                                    FieldDefinition {
                                                        name: "f",
                                                        ty: Object(
                                                            [
                                                                FieldDefinition {
                                                                    name: "f",
                                                                    ty: Object(
                                                                        [
                                                                            FieldDefinition {
                                                                                name: "f",
                                                                                ty: Object(
                                                                                    [
                                                                                        FieldDefinition {
                                                                                            name: "f",
                                                                                            ty: Object(
                                                                                                [
                                                                                                    FieldDefinition {
                                                                                                        name: "f",
                                                                                                        ty: Object(
                                                                                                            [
                                                                                                                FieldDefinition {
                                                                                                                    name: "f",
                                                                                                                    ty: Object(
                                                                                                                        [
                                                                                                                            FieldDefinition {
                                                                                                                                name: "f",
                                                                                                                                ty: Object(
                                                                                                                                    [
                                                                                                                                        FieldDefinition {
                                                                                                                                            name: "f",
                                                                                                                                            ty: Object(
                                                                                                                                                [
                                                                                                                                                    FieldDefinition {
                                                                                                                                                        name: "f",
                                                                                                                                                        ty: Object(
                                                                                                                                                            [
                                                                                                                                                                FieldDefinition {
                                                                                                                                                                    name: "f",
                                                                                                                                                                    ty: Object(
                                                                                                                                                                        [
                                                                                                                                                                            FieldDefinition {
                                                                                                                                                                                name: "f",
                                                                                                                                                                                ty: Object(
                                                                                                                                                                                    [
                                                                                                                                                                                        FieldDefinition {
                                                                                                                                                                                            name: "f",
                                                                                                                                                                                            ty: Object(
                                                                                                                                                                                                [
                                                                                                                                                                                                    FieldDefinition {
                                                                                                                                                                                                        name: "f",
                                                                                                                                                                                                        ty: Object(
                                                                                                                                                                                                            [
                                                                                                                                                                                                                FieldDefinition {
                                                                                                                                                                                                                    name: "f",
                                                                                                                                                                                                                    ty: Object(
                                                                                                                                                                                                                        [
                                                                                                                                                                                                                            FieldDefinition {
                                                                                                                                                                                                                                name: "f",
                                                                                                                                                                                                                                ty: Object(
                                                                                                                                                                                                                                    [
                                                                                                                                                                                                                                        FieldDefinition {
                                                                                                                                                                                                                                            name: "f",
                                                                                                                                                                                                                                            ty: Object(
                                                                                                                                                                                                                                                [
                                                                                                                                                                                                                                                    FieldDefinition {
                                                                                                                                                                                                                                                        name: "f",
                                                                                                                                                                                                                                                        ty: String,
                                                                                                                                                                                                                                                        optional: true,
                                                                                                                                                                                                                                                    },
                                                                                                                                                                                                                                                ],
                                                                                                                                                                                                                                            ),
                                                                                                                                                                                                                                            optional: true,
                                                                                                                                                                                                                                        },
                                                                                                                                                                                                                                    ],
                                                                                                                                                                                                                                ),
                                                                                                                                                                                                                                optional: true,
                                                                                                                                                                                                                            },
                                                                                                                                                                                                                        ],
                                                                                                                                                                                                                    ),
                                                                                                                                                                                                                    optional: true,
                                                                                                                                                                                                                },
                                                                                                                                                                                                            ],
                                                                                                                                                                                                        ),
                                                                                                                                                                                                        optional: true,
                                                                                                                                                                                                    },
                                                                                                                                                                                                ],
                                                                                                                                                                                            ),
                                                                                                                                                                                            optional: true,
                                                                                                                                                                                        },
                                                                                                                                                                                    ],
                                                                                                                                                                                ),
                                                                                                                                                                                optional: true,
                                                                                                                                                                            },
                                                                                                                                                                        ],
                                                                                                                                                                    ),
                                                                                                                                                                    optional: true,
                                                                                                                                                                },
                                                                                                                                                            ],
                                                                                                                                                        ),
                                                                                                                                                        optional: true,
                                                                                                                                                    },
                                                                                                                                                ],
                                                                                                                                            ),
                                                                                                                                            optional: true,
                                                                                                                                        },
                                                                                                                                    ],
                                                                                                                                ),
                                                                                                                                optional: true,
                                                                                                                            },
                                                                                                                        ],
                                                                                                                    ),
                                                                                                                    optional: true,
                                                                                                                },
                                                                                                            ],
                                                                                                        ),
                                                                                                        optional: true,
                                                                                                    },
                                                                                                ],
                                                                                            ),
                                                                                            optional: true,
                                                                                        },
                                                                                    ],
                                                                                ),
                                                                                optional: true,
                                                                            },
                                                                        ],
                                                                    ),
                                                                    optional: true,
                                                                },
                                                            ],
                                                        ),
                                                        optional: true,
                                                    },
                                                ],
                                            ),
                                            optional: true,
                                        },
                                    ],
                                ),
                                optional: true,
                            },
                        ],
                    ),
                    optional: false,
                    binding: false,
                },
            ],
        },
        ComponentDefinition {
            name: "Other",
            description: "",
            props: [],
        },
    ],
    component_groups: None,
    functions: [],
    actions: [],
    root: None,
    id: None,
}"####,
    ),
    (
        "object-chain-20",
        "library",
        "compact",
        r####"DefinedLibrary { definition: LibraryDefinition { components: [ComponentDefinition { name: "Deep", description: "", props: [PropDefinition { name: "v", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: String, optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: false, binding: false }] }, ComponentDefinition { name: "Other", description: "", props: [] }], component_groups: None, functions: [], actions: [], root: None, id: None }, components: [0, 1] }"####,
    ),
    (
        "object-chain-20",
        "library",
        "pretty",
        r####"DefinedLibrary {
    definition: LibraryDefinition {
        components: [
            ComponentDefinition {
                name: "Deep",
                description: "",
                props: [
                    PropDefinition {
                        name: "v",
                        ty: Object(
                            [
                                FieldDefinition {
                                    name: "f",
                                    ty: Object(
                                        [
                                            FieldDefinition {
                                                name: "f",
                                                ty: Object(
                                                    [
                                                        FieldDefinition {
                                                            name: "f",
                                                            ty: Object(
                                                                [
                                                                    FieldDefinition {
                                                                        name: "f",
                                                                        ty: Object(
                                                                            [
                                                                                FieldDefinition {
                                                                                    name: "f",
                                                                                    ty: Object(
                                                                                        [
                                                                                            FieldDefinition {
                                                                                                name: "f",
                                                                                                ty: Object(
                                                                                                    [
                                                                                                        FieldDefinition {
                                                                                                            name: "f",
                                                                                                            ty: Object(
                                                                                                                [
                                                                                                                    FieldDefinition {
                                                                                                                        name: "f",
                                                                                                                        ty: Object(
                                                                                                                            [
                                                                                                                                FieldDefinition {
                                                                                                                                    name: "f",
                                                                                                                                    ty: Object(
                                                                                                                                        [
                                                                                                                                            FieldDefinition {
                                                                                                                                                name: "f",
                                                                                                                                                ty: Object(
                                                                                                                                                    [
                                                                                                                                                        FieldDefinition {
                                                                                                                                                            name: "f",
                                                                                                                                                            ty: Object(
                                                                                                                                                                [
                                                                                                                                                                    FieldDefinition {
                                                                                                                                                                        name: "f",
                                                                                                                                                                        ty: Object(
                                                                                                                                                                            [
                                                                                                                                                                                FieldDefinition {
                                                                                                                                                                                    name: "f",
                                                                                                                                                                                    ty: Object(
                                                                                                                                                                                        [
                                                                                                                                                                                            FieldDefinition {
                                                                                                                                                                                                name: "f",
                                                                                                                                                                                                ty: Object(
                                                                                                                                                                                                    [
                                                                                                                                                                                                        FieldDefinition {
                                                                                                                                                                                                            name: "f",
                                                                                                                                                                                                            ty: Object(
                                                                                                                                                                                                                [
                                                                                                                                                                                                                    FieldDefinition {
                                                                                                                                                                                                                        name: "f",
                                                                                                                                                                                                                        ty: Object(
                                                                                                                                                                                                                            [
                                                                                                                                                                                                                                FieldDefinition {
                                                                                                                                                                                                                                    name: "f",
                                                                                                                                                                                                                                    ty: Object(
                                                                                                                                                                                                                                        [
                                                                                                                                                                                                                                            FieldDefinition {
                                                                                                                                                                                                                                                name: "f",
                                                                                                                                                                                                                                                ty: Object(
                                                                                                                                                                                                                                                    [
                                                                                                                                                                                                                                                        FieldDefinition {
                                                                                                                                                                                                                                                            name: "f",
                                                                                                                                                                                                                                                            ty: String,
                                                                                                                                                                                                                                                            optional: true,
                                                                                                                                                                                                                                                        },
                                                                                                                                                                                                                                                    ],
                                                                                                                                                                                                                                                ),
                                                                                                                                                                                                                                                optional: true,
                                                                                                                                                                                                                                            },
                                                                                                                                                                                                                                        ],
                                                                                                                                                                                                                                    ),
                                                                                                                                                                                                                                    optional: true,
                                                                                                                                                                                                                                },
                                                                                                                                                                                                                            ],
                                                                                                                                                                                                                        ),
                                                                                                                                                                                                                        optional: true,
                                                                                                                                                                                                                    },
                                                                                                                                                                                                                ],
                                                                                                                                                                                                            ),
                                                                                                                                                                                                            optional: true,
                                                                                                                                                                                                        },
                                                                                                                                                                                                    ],
                                                                                                                                                                                                ),
                                                                                                                                                                                                optional: true,
                                                                                                                                                                                            },
                                                                                                                                                                                        ],
                                                                                                                                                                                    ),
                                                                                                                                                                                    optional: true,
                                                                                                                                                                                },
                                                                                                                                                                            ],
                                                                                                                                                                        ),
                                                                                                                                                                        optional: true,
                                                                                                                                                                    },
                                                                                                                                                                ],
                                                                                                                                                            ),
                                                                                                                                                            optional: true,
                                                                                                                                                        },
                                                                                                                                                    ],
                                                                                                                                                ),
                                                                                                                                                optional: true,
                                                                                                                                            },
                                                                                                                                        ],
                                                                                                                                    ),
                                                                                                                                    optional: true,
                                                                                                                                },
                                                                                                                            ],
                                                                                                                        ),
                                                                                                                        optional: true,
                                                                                                                    },
                                                                                                                ],
                                                                                                            ),
                                                                                                            optional: true,
                                                                                                        },
                                                                                                    ],
                                                                                                ),
                                                                                                optional: true,
                                                                                            },
                                                                                        ],
                                                                                    ),
                                                                                    optional: true,
                                                                                },
                                                                            ],
                                                                        ),
                                                                        optional: true,
                                                                    },
                                                                ],
                                                            ),
                                                            optional: true,
                                                        },
                                                    ],
                                                ),
                                                optional: true,
                                            },
                                        ],
                                    ),
                                    optional: true,
                                },
                            ],
                        ),
                        optional: false,
                        binding: false,
                    },
                ],
            },
            ComponentDefinition {
                name: "Other",
                description: "",
                props: [],
            },
        ],
        component_groups: None,
        functions: [],
        actions: [],
        root: None,
        id: None,
    },
    components: [
        0,
        1,
    ],
}"####,
    ),
    (
        "array-chain-20",
        "type",
        "compact",
        r####"Array(Array(Array(Array(Array(Array(Array(Array(Array(Array(Array(Array(Array(Array(Array(Array(Array(Array(Array(String)))))))))))))))))))"####,
    ),
    (
        "array-chain-20",
        "type",
        "pretty",
        r####"Array(
    Array(
        Array(
            Array(
                Array(
                    Array(
                        Array(
                            Array(
                                Array(
                                    Array(
                                        Array(
                                            Array(
                                                Array(
                                                    Array(
                                                        Array(
                                                            Array(
                                                                Array(
                                                                    Array(
                                                                        Array(
                                                                            String,
                                                                        ),
                                                                    ),
                                                                ),
                                                            ),
                                                        ),
                                                    ),
                                                ),
                                            ),
                                        ),
                                    ),
                                ),
                            ),
                        ),
                    ),
                ),
            ),
        ),
    ),
)"####,
    ),
    (
        "union-chain-20",
        "type",
        "compact",
        r####"Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, String])])])])])])])])])])])])])])])])])])])"####,
    ),
    (
        "union-chain-20",
        "type",
        "pretty",
        r####"Union(
    [
        Number,
        Union(
            [
                Number,
                Union(
                    [
                        Number,
                        Union(
                            [
                                Number,
                                Union(
                                    [
                                        Number,
                                        Union(
                                            [
                                                Number,
                                                Union(
                                                    [
                                                        Number,
                                                        Union(
                                                            [
                                                                Number,
                                                                Union(
                                                                    [
                                                                        Number,
                                                                        Union(
                                                                            [
                                                                                Number,
                                                                                Union(
                                                                                    [
                                                                                        Number,
                                                                                        Union(
                                                                                            [
                                                                                                Number,
                                                                                                Union(
                                                                                                    [
                                                                                                        Number,
                                                                                                        Union(
                                                                                                            [
                                                                                                                Number,
                                                                                                                Union(
                                                                                                                    [
                                                                                                                        Number,
                                                                                                                        Union(
                                                                                                                            [
                                                                                                                                Number,
                                                                                                                                Union(
                                                                                                                                    [
                                                                                                                                        Number,
                                                                                                                                        Union(
                                                                                                                                            [
                                                                                                                                                Number,
                                                                                                                                                Union(
                                                                                                                                                    [
                                                                                                                                                        Number,
                                                                                                                                                        String,
                                                                                                                                                    ],
                                                                                                                                                ),
                                                                                                                                            ],
                                                                                                                                        ),
                                                                                                                                    ],
                                                                                                                                ),
                                                                                                                            ],
                                                                                                                        ),
                                                                                                                    ],
                                                                                                                ),
                                                                                                            ],
                                                                                                        ),
                                                                                                    ],
                                                                                                ),
                                                                                            ],
                                                                                        ),
                                                                                    ],
                                                                                ),
                                                                            ],
                                                                        ),
                                                                    ],
                                                                ),
                                                            ],
                                                        ),
                                                    ],
                                                ),
                                            ],
                                        ),
                                    ],
                                ),
                            ],
                        ),
                    ],
                ),
            ],
        ),
    ],
)"####,
    ),
    (
        "union-chain-20",
        "definition",
        "compact",
        r####"LibraryDefinition { components: [ComponentDefinition { name: "Deep", description: "", props: [PropDefinition { name: "v", ty: Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, String])])])])])])])])])])])])])])])])])])]), optional: false, binding: false }] }, ComponentDefinition { name: "Other", description: "", props: [] }], component_groups: None, functions: [], actions: [], root: None, id: None }"####,
    ),
    (
        "union-chain-20",
        "definition",
        "pretty",
        r####"LibraryDefinition {
    components: [
        ComponentDefinition {
            name: "Deep",
            description: "",
            props: [
                PropDefinition {
                    name: "v",
                    ty: Union(
                        [
                            Number,
                            Union(
                                [
                                    Number,
                                    Union(
                                        [
                                            Number,
                                            Union(
                                                [
                                                    Number,
                                                    Union(
                                                        [
                                                            Number,
                                                            Union(
                                                                [
                                                                    Number,
                                                                    Union(
                                                                        [
                                                                            Number,
                                                                            Union(
                                                                                [
                                                                                    Number,
                                                                                    Union(
                                                                                        [
                                                                                            Number,
                                                                                            Union(
                                                                                                [
                                                                                                    Number,
                                                                                                    Union(
                                                                                                        [
                                                                                                            Number,
                                                                                                            Union(
                                                                                                                [
                                                                                                                    Number,
                                                                                                                    Union(
                                                                                                                        [
                                                                                                                            Number,
                                                                                                                            Union(
                                                                                                                                [
                                                                                                                                    Number,
                                                                                                                                    Union(
                                                                                                                                        [
                                                                                                                                            Number,
                                                                                                                                            Union(
                                                                                                                                                [
                                                                                                                                                    Number,
                                                                                                                                                    Union(
                                                                                                                                                        [
                                                                                                                                                            Number,
                                                                                                                                                            Union(
                                                                                                                                                                [
                                                                                                                                                                    Number,
                                                                                                                                                                    Union(
                                                                                                                                                                        [
                                                                                                                                                                            Number,
                                                                                                                                                                            String,
                                                                                                                                                                        ],
                                                                                                                                                                    ),
                                                                                                                                                                ],
                                                                                                                                                            ),
                                                                                                                                                        ],
                                                                                                                                                    ),
                                                                                                                                                ],
                                                                                                                                            ),
                                                                                                                                        ],
                                                                                                                                    ),
                                                                                                                                ],
                                                                                                                            ),
                                                                                                                        ],
                                                                                                                    ),
                                                                                                                ],
                                                                                                            ),
                                                                                                        ],
                                                                                                    ),
                                                                                                ],
                                                                                            ),
                                                                                        ],
                                                                                    ),
                                                                                ],
                                                                            ),
                                                                        ],
                                                                    ),
                                                                ],
                                                            ),
                                                        ],
                                                    ),
                                                ],
                                            ),
                                        ],
                                    ),
                                ],
                            ),
                        ],
                    ),
                    optional: false,
                    binding: false,
                },
            ],
        },
        ComponentDefinition {
            name: "Other",
            description: "",
            props: [],
        },
    ],
    component_groups: None,
    functions: [],
    actions: [],
    root: None,
    id: None,
}"####,
    ),
    (
        "union-chain-20",
        "library",
        "compact",
        r####"DefinedLibrary { definition: LibraryDefinition { components: [ComponentDefinition { name: "Deep", description: "", props: [PropDefinition { name: "v", ty: Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, Union([Number, String])])])])])])])])])])])])])])])])])])]), optional: false, binding: false }] }, ComponentDefinition { name: "Other", description: "", props: [] }], component_groups: None, functions: [], actions: [], root: None, id: None }, components: [0, 1] }"####,
    ),
    (
        "union-chain-20",
        "library",
        "pretty",
        r####"DefinedLibrary {
    definition: LibraryDefinition {
        components: [
            ComponentDefinition {
                name: "Deep",
                description: "",
                props: [
                    PropDefinition {
                        name: "v",
                        ty: Union(
                            [
                                Number,
                                Union(
                                    [
                                        Number,
                                        Union(
                                            [
                                                Number,
                                                Union(
                                                    [
                                                        Number,
                                                        Union(
                                                            [
                                                                Number,
                                                                Union(
                                                                    [
                                                                        Number,
                                                                        Union(
                                                                            [
                                                                                Number,
                                                                                Union(
                                                                                    [
                                                                                        Number,
                                                                                        Union(
                                                                                            [
                                                                                                Number,
                                                                                                Union(
                                                                                                    [
                                                                                                        Number,
                                                                                                        Union(
                                                                                                            [
                                                                                                                Number,
                                                                                                                Union(
                                                                                                                    [
                                                                                                                        Number,
                                                                                                                        Union(
                                                                                                                            [
                                                                                                                                Number,
                                                                                                                                Union(
                                                                                                                                    [
                                                                                                                                        Number,
                                                                                                                                        Union(
                                                                                                                                            [
                                                                                                                                                Number,
                                                                                                                                                Union(
                                                                                                                                                    [
                                                                                                                                                        Number,
                                                                                                                                                        Union(
                                                                                                                                                            [
                                                                                                                                                                Number,
                                                                                                                                                                Union(
                                                                                                                                                                    [
                                                                                                                                                                        Number,
                                                                                                                                                                        Union(
                                                                                                                                                                            [
                                                                                                                                                                                Number,
                                                                                                                                                                                String,
                                                                                                                                                                            ],
                                                                                                                                                                        ),
                                                                                                                                                                    ],
                                                                                                                                                                ),
                                                                                                                                                            ],
                                                                                                                                                        ),
                                                                                                                                                    ],
                                                                                                                                                ),
                                                                                                                                            ],
                                                                                                                                        ),
                                                                                                                                    ],
                                                                                                                                ),
                                                                                                                            ],
                                                                                                                        ),
                                                                                                                    ],
                                                                                                                ),
                                                                                                            ],
                                                                                                        ),
                                                                                                    ],
                                                                                                ),
                                                                                            ],
                                                                                        ),
                                                                                    ],
                                                                                ),
                                                                            ],
                                                                        ),
                                                                    ],
                                                                ),
                                                            ],
                                                        ),
                                                    ],
                                                ),
                                            ],
                                        ),
                                    ],
                                ),
                            ],
                        ),
                        optional: false,
                        binding: false,
                    },
                ],
            },
            ComponentDefinition {
                name: "Other",
                description: "",
                props: [],
            },
        ],
        component_groups: None,
        functions: [],
        actions: [],
        root: None,
        id: None,
    },
    components: [
        0,
        1,
    ],
}"####,
    ),
    (
        "nullable-chain-20",
        "type",
        "compact",
        r####"Nullable(Nullable(Nullable(Nullable(Nullable(Nullable(Nullable(Nullable(Nullable(Nullable(Nullable(Nullable(Nullable(Nullable(Nullable(Nullable(Nullable(Nullable(Nullable(String)))))))))))))))))))"####,
    ),
    (
        "nullable-chain-20",
        "type",
        "pretty",
        r####"Nullable(
    Nullable(
        Nullable(
            Nullable(
                Nullable(
                    Nullable(
                        Nullable(
                            Nullable(
                                Nullable(
                                    Nullable(
                                        Nullable(
                                            Nullable(
                                                Nullable(
                                                    Nullable(
                                                        Nullable(
                                                            Nullable(
                                                                Nullable(
                                                                    Nullable(
                                                                        Nullable(
                                                                            String,
                                                                        ),
                                                                    ),
                                                                ),
                                                            ),
                                                        ),
                                                    ),
                                                ),
                                            ),
                                        ),
                                    ),
                                ),
                            ),
                        ),
                    ),
                ),
            ),
        ),
    ),
)"####,
    ),
    (
        "record-chain-20",
        "type",
        "compact",
        r####"Record(Record(Record(Record(Record(Record(Record(Record(Record(Record(Record(Record(Record(Record(Record(Record(Record(Record(Record(String)))))))))))))))))))"####,
    ),
    (
        "record-chain-20",
        "type",
        "pretty",
        r####"Record(
    Record(
        Record(
            Record(
                Record(
                    Record(
                        Record(
                            Record(
                                Record(
                                    Record(
                                        Record(
                                            Record(
                                                Record(
                                                    Record(
                                                        Record(
                                                            Record(
                                                                Record(
                                                                    Record(
                                                                        Record(
                                                                            String,
                                                                        ),
                                                                    ),
                                                                ),
                                                            ),
                                                        ),
                                                    ),
                                                ),
                                            ),
                                        ),
                                    ),
                                ),
                            ),
                        ),
                    ),
                ),
            ),
        ),
    ),
)"####,
    ),
    (
        "named-chain-20",
        "type",
        "compact",
        r####"Named("N", Nullable(Named("N", Nullable(Named("N", Nullable(Named("N", Nullable(Named("N", Nullable(Named("N", Nullable(Named("N", Nullable(Named("N", Nullable(Named("N", Nullable(Named("N", String)))))))))))))))))))"####,
    ),
    (
        "named-chain-20",
        "type",
        "pretty",
        r####"Named(
    "N",
    Nullable(
        Named(
            "N",
            Nullable(
                Named(
                    "N",
                    Nullable(
                        Named(
                            "N",
                            Nullable(
                                Named(
                                    "N",
                                    Nullable(
                                        Named(
                                            "N",
                                            Nullable(
                                                Named(
                                                    "N",
                                                    Nullable(
                                                        Named(
                                                            "N",
                                                            Nullable(
                                                                Named(
                                                                    "N",
                                                                    Nullable(
                                                                        Named(
                                                                            "N",
                                                                            String,
                                                                        ),
                                                                    ),
                                                                ),
                                                            ),
                                                        ),
                                                    ),
                                                ),
                                            ),
                                        ),
                                    ),
                                ),
                            ),
                        ),
                    ),
                ),
            ),
        ),
    ),
)"####,
    ),
    (
        "min-array-chain-20",
        "type",
        "compact",
        r####"ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(ArrayWithMinimum(String, Number(2)), Number(2)), Number(2)), Number(2)), Number(2)), Number(2)), Number(2)), Number(2)), Number(2)), Number(2)), Number(2)), Number(2)), Number(2)), Number(2)), Number(2)), Number(2)), Number(2)), Number(2)), Number(2))"####,
    ),
    (
        "min-array-chain-20",
        "type",
        "pretty",
        r####"ArrayWithMinimum(
    ArrayWithMinimum(
        ArrayWithMinimum(
            ArrayWithMinimum(
                ArrayWithMinimum(
                    ArrayWithMinimum(
                        ArrayWithMinimum(
                            ArrayWithMinimum(
                                ArrayWithMinimum(
                                    ArrayWithMinimum(
                                        ArrayWithMinimum(
                                            ArrayWithMinimum(
                                                ArrayWithMinimum(
                                                    ArrayWithMinimum(
                                                        ArrayWithMinimum(
                                                            ArrayWithMinimum(
                                                                ArrayWithMinimum(
                                                                    ArrayWithMinimum(
                                                                        ArrayWithMinimum(
                                                                            String,
                                                                            Number(2),
                                                                        ),
                                                                        Number(2),
                                                                    ),
                                                                    Number(2),
                                                                ),
                                                                Number(2),
                                                            ),
                                                            Number(2),
                                                        ),
                                                        Number(2),
                                                    ),
                                                    Number(2),
                                                ),
                                                Number(2),
                                            ),
                                            Number(2),
                                        ),
                                        Number(2),
                                    ),
                                    Number(2),
                                ),
                                Number(2),
                            ),
                            Number(2),
                        ),
                        Number(2),
                    ),
                    Number(2),
                ),
                Number(2),
            ),
            Number(2),
        ),
        Number(2),
    ),
    Number(2),
)"####,
    ),
    (
        "object-chain-20-differing-leaf",
        "type",
        "compact",
        r####"Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Object([FieldDefinition { name: "f", ty: Boolean, optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }]), optional: true }])"####,
    ),
    (
        "object-chain-20-differing-leaf",
        "type",
        "pretty",
        r####"Object(
    [
        FieldDefinition {
            name: "f",
            ty: Object(
                [
                    FieldDefinition {
                        name: "f",
                        ty: Object(
                            [
                                FieldDefinition {
                                    name: "f",
                                    ty: Object(
                                        [
                                            FieldDefinition {
                                                name: "f",
                                                ty: Object(
                                                    [
                                                        FieldDefinition {
                                                            name: "f",
                                                            ty: Object(
                                                                [
                                                                    FieldDefinition {
                                                                        name: "f",
                                                                        ty: Object(
                                                                            [
                                                                                FieldDefinition {
                                                                                    name: "f",
                                                                                    ty: Object(
                                                                                        [
                                                                                            FieldDefinition {
                                                                                                name: "f",
                                                                                                ty: Object(
                                                                                                    [
                                                                                                        FieldDefinition {
                                                                                                            name: "f",
                                                                                                            ty: Object(
                                                                                                                [
                                                                                                                    FieldDefinition {
                                                                                                                        name: "f",
                                                                                                                        ty: Object(
                                                                                                                            [
                                                                                                                                FieldDefinition {
                                                                                                                                    name: "f",
                                                                                                                                    ty: Object(
                                                                                                                                        [
                                                                                                                                            FieldDefinition {
                                                                                                                                                name: "f",
                                                                                                                                                ty: Object(
                                                                                                                                                    [
                                                                                                                                                        FieldDefinition {
                                                                                                                                                            name: "f",
                                                                                                                                                            ty: Object(
                                                                                                                                                                [
                                                                                                                                                                    FieldDefinition {
                                                                                                                                                                        name: "f",
                                                                                                                                                                        ty: Object(
                                                                                                                                                                            [
                                                                                                                                                                                FieldDefinition {
                                                                                                                                                                                    name: "f",
                                                                                                                                                                                    ty: Object(
                                                                                                                                                                                        [
                                                                                                                                                                                            FieldDefinition {
                                                                                                                                                                                                name: "f",
                                                                                                                                                                                                ty: Object(
                                                                                                                                                                                                    [
                                                                                                                                                                                                        FieldDefinition {
                                                                                                                                                                                                            name: "f",
                                                                                                                                                                                                            ty: Object(
                                                                                                                                                                                                                [
                                                                                                                                                                                                                    FieldDefinition {
                                                                                                                                                                                                                        name: "f",
                                                                                                                                                                                                                        ty: Object(
                                                                                                                                                                                                                            [
                                                                                                                                                                                                                                FieldDefinition {
                                                                                                                                                                                                                                    name: "f",
                                                                                                                                                                                                                                    ty: Boolean,
                                                                                                                                                                                                                                    optional: true,
                                                                                                                                                                                                                                },
                                                                                                                                                                                                                            ],
                                                                                                                                                                                                                        ),
                                                                                                                                                                                                                        optional: true,
                                                                                                                                                                                                                    },
                                                                                                                                                                                                                ],
                                                                                                                                                                                                            ),
                                                                                                                                                                                                            optional: true,
                                                                                                                                                                                                        },
                                                                                                                                                                                                    ],
                                                                                                                                                                                                ),
                                                                                                                                                                                                optional: true,
                                                                                                                                                                                            },
                                                                                                                                                                                        ],
                                                                                                                                                                                    ),
                                                                                                                                                                                    optional: true,
                                                                                                                                                                                },
                                                                                                                                                                            ],
                                                                                                                                                                        ),
                                                                                                                                                                        optional: true,
                                                                                                                                                                    },
                                                                                                                                                                ],
                                                                                                                                                            ),
                                                                                                                                                            optional: true,
                                                                                                                                                        },
                                                                                                                                                    ],
                                                                                                                                                ),
                                                                                                                                                optional: true,
                                                                                                                                            },
                                                                                                                                        ],
                                                                                                                                    ),
                                                                                                                                    optional: true,
                                                                                                                                },
                                                                                                                            ],
                                                                                                                        ),
                                                                                                                        optional: true,
                                                                                                                    },
                                                                                                                ],
                                                                                                            ),
                                                                                                            optional: true,
                                                                                                        },
                                                                                                    ],
                                                                                                ),
                                                                                                optional: true,
                                                                                            },
                                                                                        ],
                                                                                    ),
                                                                                    optional: true,
                                                                                },
                                                                            ],
                                                                        ),
                                                                        optional: true,
                                                                    },
                                                                ],
                                                            ),
                                                            optional: true,
                                                        },
                                                    ],
                                                ),
                                                optional: true,
                                            },
                                        ],
                                    ),
                                    optional: true,
                                },
                            ],
                        ),
                        optional: true,
                    },
                ],
            ),
            optional: true,
        },
    ],
)"####,
    ),
    (
        "json-default-6",
        "type",
        "compact",
        r####"Default(String, DefaultValue(Array [Object {"key": Array [Object {"key": Array [Object {"key": Array [Object {"key": Array [Object {"key": Array [Object {"key": String("leaf")}]}]}]}]}]}]))"####,
    ),
    (
        "json-default-6",
        "type",
        "pretty",
        r####"Default(
    String,
    DefaultValue(
        Array [
            Object {
                "key": Array [
                    Object {
                        "key": Array [
                            Object {
                                "key": Array [
                                    Object {
                                        "key": Array [
                                            Object {
                                                "key": Array [
                                                    Object {
                                                        "key": String("leaf"),
                                                    },
                                                ],
                                            },
                                        ],
                                    },
                                ],
                            },
                        ],
                    },
                ],
            },
        ],
    ),
)"####,
    ),
    (
        "json-default-6",
        "definition",
        "compact",
        r####"LibraryDefinition { components: [ComponentDefinition { name: "Deep", description: "", props: [PropDefinition { name: "v", ty: Default(String, DefaultValue(Array [Object {"key": Array [Object {"key": Array [Object {"key": Array [Object {"key": Array [Object {"key": Array [Object {"key": String("leaf")}]}]}]}]}]}])), optional: false, binding: false }] }, ComponentDefinition { name: "Other", description: "", props: [] }], component_groups: None, functions: [], actions: [], root: None, id: None }"####,
    ),
    (
        "json-default-6",
        "definition",
        "pretty",
        r####"LibraryDefinition {
    components: [
        ComponentDefinition {
            name: "Deep",
            description: "",
            props: [
                PropDefinition {
                    name: "v",
                    ty: Default(
                        String,
                        DefaultValue(
                            Array [
                                Object {
                                    "key": Array [
                                        Object {
                                            "key": Array [
                                                Object {
                                                    "key": Array [
                                                        Object {
                                                            "key": Array [
                                                                Object {
                                                                    "key": Array [
                                                                        Object {
                                                                            "key": String("leaf"),
                                                                        },
                                                                    ],
                                                                },
                                                            ],
                                                        },
                                                    ],
                                                },
                                            ],
                                        },
                                    ],
                                },
                            ],
                        ),
                    ),
                    optional: false,
                    binding: false,
                },
            ],
        },
        ComponentDefinition {
            name: "Other",
            description: "",
            props: [],
        },
    ],
    component_groups: None,
    functions: [],
    actions: [],
    root: None,
    id: None,
}"####,
    ),
    (
        "json-default-6",
        "library",
        "compact",
        r####"DefinedLibrary { definition: LibraryDefinition { components: [ComponentDefinition { name: "Deep", description: "", props: [PropDefinition { name: "v", ty: Default(String, DefaultValue(Array [Object {"key": Array [Object {"key": Array [Object {"key": Array [Object {"key": Array [Object {"key": Array [Object {"key": String("leaf")}]}]}]}]}]}])), optional: false, binding: false }] }, ComponentDefinition { name: "Other", description: "", props: [] }], component_groups: None, functions: [], actions: [], root: None, id: None }, components: [0, 1] }"####,
    ),
    (
        "json-default-6",
        "library",
        "pretty",
        r####"DefinedLibrary {
    definition: LibraryDefinition {
        components: [
            ComponentDefinition {
                name: "Deep",
                description: "",
                props: [
                    PropDefinition {
                        name: "v",
                        ty: Default(
                            String,
                            DefaultValue(
                                Array [
                                    Object {
                                        "key": Array [
                                            Object {
                                                "key": Array [
                                                    Object {
                                                        "key": Array [
                                                            Object {
                                                                "key": Array [
                                                                    Object {
                                                                        "key": Array [
                                                                            Object {
                                                                                "key": String("leaf"),
                                                                            },
                                                                        ],
                                                                    },
                                                                ],
                                                            },
                                                        ],
                                                    },
                                                ],
                                            },
                                        ],
                                    },
                                ],
                            ),
                        ),
                        optional: false,
                        binding: false,
                    },
                ],
            },
            ComponentDefinition {
                name: "Other",
                description: "",
                props: [],
            },
        ],
        component_groups: None,
        functions: [],
        actions: [],
        root: None,
        id: None,
    },
    components: [
        0,
        1,
    ],
}"####,
    ),
];
