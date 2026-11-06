/// Plan 12 B2 — Map all 87 SCIP Kind values → graphrelay SymbolKind.
/// Verified against scip-0.8.1/src/generated/scip.rs (all 87 variants listed).
/// Kinds with no graphrelay equivalent return None — existing kind must NOT be overwritten.
#[cfg(feature = "scip")]
use graphrelay_core::types::SymbolKind;

#[cfg(feature = "scip")]
use scip::types::symbol_information::Kind as ScipKind;

/// Returns `Some(SymbolKind)` when SCIP provides a clear graphrelay mapping,
/// `None` for kinds that have no equivalent (primitive types, parameters, etc.).
#[cfg(feature = "scip")]
pub fn scip_kind_to_symbol_kind(kind: ScipKind) -> Option<SymbolKind> {
    use ScipKind::*;
    match kind {
        // Direct mappings
        Class                                       => Some(SymbolKind::Class),
        Constant                                    => Some(SymbolKind::Const),
        Constructor                                 => Some(SymbolKind::Function),
        Enum                                        => Some(SymbolKind::Enum),
        EnumMember                                  => Some(SymbolKind::Enum),
        Function                                    => Some(SymbolKind::Function),
        Interface                                   => Some(SymbolKind::Interface),
        Method                                      => Some(SymbolKind::Method),
        Module                                      => Some(SymbolKind::Module),
        Namespace                                   => Some(SymbolKind::Module),
        Package                                     => Some(SymbolKind::Module),
        PackageObject                               => Some(SymbolKind::Module),
        Protocol                                    => Some(SymbolKind::Trait),
        Struct                                      => Some(SymbolKind::Struct),
        Trait                                       => Some(SymbolKind::Trait),
        TypeAlias                                   => Some(SymbolKind::TypeAlias),
        TypeParameter                               => Some(SymbolKind::TypeAlias),
        TypeClass                                   => Some(SymbolKind::Trait),
        Union                                       => Some(SymbolKind::Struct),
        // Method variants → Method
        AbstractMethod
        | MethodAlias
        | MethodReceiver
        | MethodSpecification
        | ProtocolMethod
        | PureVirtualMethod
        | SingletonMethod
        | StaticMethod
        | TraitMethod
        | TypeClassMethod                           => Some(SymbolKind::Method),
        // Property / accessor-like → Function (closest mapping)
        Accessor | Delegate | Getter | Property
        | Setter | Subscript | StaticProperty      => Some(SymbolKind::Function),
        // Field-like → Function (stored as a callable entity)
        Field | StaticDataMember | StaticField     => Some(SymbolKind::Function),
        // Module variants
        Extension | Mixin | SingletonClass          => Some(SymbolKind::Module),
        // No meaningful graphrelay equivalent — do NOT overwrite tree-sitter kind
        UnspecifiedKind
        | Array | Assertion | AssociatedType
        | Attribute | Axiom | Boolean | Concept
        | Contract | DataFamily | Error | Event
        | Fact | File | Grammar | Instance
        | Key | Lang | Lemma | Library | Macro
        | Message | Modifier | Null | Number
        | Object | Operator | Pattern | Predicate
        | Quasiquoter | SelfParameter | Signature
        | String | Tactic | Theorem | ThisParameter
        | Type | TypeFamily | Value | Variable
        | Parameter | ParameterLabel
        | StaticEvent | StaticVariable            => None,
    }
}
