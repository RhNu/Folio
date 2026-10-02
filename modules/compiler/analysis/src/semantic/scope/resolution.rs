//! Name and member lookup with the existing binding precedence.
use super::*;

impl<'a> Scope<'a> {
    pub(super) fn resolve_name(&mut self, name: &NameRef) -> (Type, Option<Binding>) {
        if let Some(local) = self.locals.get(&key(&name.text)) {
            return (
                local.ty.clone(),
                Some(Binding {
                    name: name.clone(),
                    symbol: local.symbol.clone(),
                    definition: Some(local.definition),
                }),
            );
        }
        if name.text.eq_ignore_ascii_case("self") {
            return (
                Type::Script(self.script.name.clone()),
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::Script(self.script.name.clone()),
                    definition: self.script.definition,
                }),
            );
        }
        if name.text.eq_ignore_ascii_case("parent")
            && let Some(parent) = &self.script.parent
        {
            return (
                Type::Script(parent.clone()),
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::ParentReceiver {
                        script: parent.clone(),
                    },
                    definition: self
                        .world
                        .scripts
                        .get(&key(parent))
                        .and_then(|script| script.definition),
                }),
            );
        }
        // These methods are emitted by the compiler and have fixed signatures;
        // External declarations cannot redefine their meaning.
        if let Some(ty) = state_runtime_intrinsic_type(&name.text) {
            if self.member.global {
                self.issue(
                    "semantic.instance-member",
                    "state runtime methods require an instance",
                    name.span,
                );
                return (Type::Error, None);
            }
            return (
                ty,
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::Intrinsic {
                        name: name.text.clone(),
                    },
                    definition: None,
                }),
            );
        }
        if let Some(state) = &self.state
            && let Some((owner, member)) =
                lookup_state_member(self.world, &self.script.name, state, &name.text)
        {
            return (
                member.ty.clone(),
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::StateMember {
                        script: owner.name.clone(),
                        state: state.clone(),
                        name: member.name.clone(),
                    },
                    definition: member.definition,
                }),
            );
        }
        if let Some((owner, member)) = lookup_member(self.world, &self.script.name, &name.text) {
            return (
                member.ty.clone(),
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::Member {
                        script: owner.name.clone(),
                        name: member.name.clone(),
                    },
                    definition: member.definition,
                }),
            );
        }
        if let Some(script) = self.world.scripts.get(&key(&name.text)) {
            return (
                Type::Script(script.name.clone()),
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::Script(script.name.clone()),
                    definition: script.definition,
                }),
            );
        }
        let mut imported = self
            .imports
            .iter()
            .filter_map(|script| lookup_member(self.world, script, &name.text))
            .filter(|(_, member)| member.global)
            .collect::<Vec<_>>();
        if imported.len() == 1 {
            let (owner, member) = imported.pop().unwrap();
            return (
                member.ty.clone(),
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::Member {
                        script: owner.name.clone(),
                        name: member.name.clone(),
                    },
                    definition: member.definition,
                }),
            );
        }
        if imported.len() > 1 {
            self.issue(
                "semantic.ambiguous-import",
                format!("ambiguous imported member {}", name.text),
                name.span,
            );
        } else {
            self.issue(
                "semantic.unknown-name",
                format!("unknown name {}", name.text),
                name.span,
            );
        }
        (Type::Error, None)
    }

    pub(super) fn resolve_member(
        &mut self,
        owner: &ExpressionFact,
        name: &NameRef,
    ) -> (Type, Option<Binding>) {
        if let Type::Array(_) = &owner.ty
            && let Some(signature) = crate::intrinsic_signature(&name.text, Some(&owner.ty))
        {
            return (
                signature.result,
                signature.callable.then(|| Binding {
                    name: name.clone(),
                    symbol: Symbol::Intrinsic {
                        name: name.text.clone(),
                    },
                    definition: None,
                }),
            );
        }
        let Type::Script(script_name) = &owner.ty else {
            if owner.ty != Type::Error {
                self.issue(
                    "semantic.no-member",
                    format!("type {:?} has no member {}", owner.ty, name.text),
                    name.span,
                );
            }
            return (Type::Error, None);
        };
        if let Some(ty) = state_runtime_intrinsic_type(&name.text) {
            let static_script = matches!(
                owner.binding.as_ref().map(|binding| &binding.symbol),
                Some(Symbol::Script(_))
            ) && !matches!(&owner.kind, ExpressionKind::Reference(reference) if reference.text.eq_ignore_ascii_case("self") || reference.text.eq_ignore_ascii_case("parent"));
            if static_script {
                self.issue(
                    "semantic.instance-member",
                    "state runtime methods require an instance",
                    name.span,
                );
                return (Type::Error, None);
            }
            return (
                ty,
                Some(Binding {
                    name: name.clone(),
                    symbol: Symbol::Intrinsic {
                        name: name.text.clone(),
                    },
                    definition: None,
                }),
            );
        }
        let Some((script, member)) = lookup_member(self.world, script_name, &name.text) else {
            if self.world.scripts.contains_key(&key(script_name)) {
                self.issue(
                    "semantic.unknown-member",
                    format!("unknown member {}", name.text),
                    name.span,
                );
            }
            return (Type::Error, None);
        };
        if matches!(
            owner.binding.as_ref().map(|binding| &binding.symbol),
            Some(Symbol::ParentReceiver { .. })
        ) && !matches!(
            member.kind,
            MemberKind::Function | MemberKind::Event | MemberKind::UnknownCallable
        ) {
            self.issue(
                "semantic.parent-context",
                "Parent is only a function call receiver",
                name.span,
            );
            return (Type::Error, None);
        }
        let static_script = matches!(
            owner.binding.as_ref().map(|binding| &binding.symbol),
            Some(Symbol::Script(_))
        ) && !matches!(&owner.kind, ExpressionKind::Reference(reference) if reference.text.eq_ignore_ascii_case("self") || reference.text.eq_ignore_ascii_case("parent"));
        if static_script
            && !member.global
            && matches!(
                member.kind,
                MemberKind::Function | MemberKind::UnknownCallable
            )
        {
            self.issue(
                "semantic.instance-member",
                format!("{} requires an instance", name.text),
                name.span,
            );
        }
        if !static_script
            && member.global
            && matches!(
                member.kind,
                MemberKind::Function | MemberKind::UnknownCallable
            )
        {
            self.issue(
                "semantic.global-member",
                "global function requires a script qualifier",
                name.span,
            );
            return (Type::Error, None);
        }
        (
            member.ty.clone(),
            Some(Binding {
                name: name.clone(),
                symbol: Symbol::Member {
                    script: script.name.clone(),
                    name: member.name.clone(),
                },
                definition: member.definition,
            }),
        )
    }
}
