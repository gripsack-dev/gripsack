//! Frozen virtual requirements use the same MatchSpec grammar and version/build
//! semantics as the resolver. `constrains` restricts an existing fact; unlike
//! `depends`, it never manufactures a requirement that an absent fact exist.
use crate::records::{self,RecordError};
use gripsack_ir::workspace_v6::lock::{LockedCondaPackage,LockedVirtualPackage};
use rattler_conda_types::{GenericVirtualPackage,MatchSpec,MatchSpecCondition,Matches,PackageName,ParseMatchSpecOptions,ParseStrictness,RepoDataRecord,Version};
use std::{collections::BTreeSet,str::FromStr};

#[derive(Debug,thiserror::Error)]
pub enum VirtualConstraintError {
    #[error(transparent)]
    Record(#[from] RecordError),
    #[error("invalid measured virtual package {name:?}: {detail}")]
    Fact { name:String,detail:String },
    #[error("package {package:?}: invalid constraint {spec:?}: {detail}")]
    Spec { package:String,spec:String,detail:String },
    #[error("package {package:?}: required virtual package {name:?} is absent for {spec:?}")]
    Missing { package:String,spec:String,name:String },
    #[error("package {package:?}: {spec:?} is not satisfied by measured {name}={version}={build}")]
    Unsatisfied { package:String,spec:String,name:String,version:String,build:String },
}

#[derive(Clone,Copy)]
enum Requirement {
    Dependency,
    Constraint,
}

/// `facts` is the complete measured name/version/build set. Frozen values used
/// during solving are not host facts and must not be passed as substitutes.
pub fn evaluate_virtual_constraints(packages: &[LockedCondaPackage],facts: &[LockedVirtualPackage]) -> Result<(),VirtualConstraintError> {
    let mut names=BTreeSet::new();
    let mut virtuals=Vec::with_capacity(facts.len());
    for fact in facts {
        let fail=|detail:String| VirtualConstraintError::Fact { name:fact.name.clone(),detail };
        if !fact.name.starts_with("__") || !names.insert(fact.name.as_str()) {
            return Err(fail("not a unique virtual-package name".into()));
        }
        virtuals.push(GenericVirtualPackage {
            name:PackageName::from_str(&fact.name).map_err(|error| fail(error.to_string()))?,
            version:Version::from_str(&fact.version).map_err(|error| fail(error.to_string()))?,
            build_string:fact.build.clone(),
        });
    }
    let mut environment=None;
    for package in packages {
        for (requirements,kind) in [(&package.depends,Requirement::Dependency),(&package.constrains,Requirement::Constraint)] {
            for text in requirements {
                let spec=MatchSpec::from_str(text,ParseMatchSpecOptions::from(ParseStrictness::Lenient).with_conditionals(true))
                    .map_err(|error| invalid_spec(package,text,error.to_string()))?;
                let name=spec.name.as_exact().ok_or_else(|| invalid_spec(package,text,"dependency records require an exact package name".into()))?;
                if !name.as_normalized().starts_with("__") { continue; }
                if let Some(condition)=&spec.condition {
                    if environment.is_none() {
                        environment=Some(packages.iter().map(records::repository_record).collect::<Result<Vec<_>,_>>()?);
                    }
                    if !condition_holds(condition,environment.as_deref().expect("initialized"),&virtuals) { continue; }
                }
                let measured=virtuals.iter().find(|fact| &fact.name==name);
                let Some(measured)=measured else {
                    if matches!(kind,Requirement::Constraint) { continue; }
                    return Err(VirtualConstraintError::Missing { package:package.name.clone(),spec:text.clone(),name:name.as_normalized().into() });
                };
                if !spec.matches(measured) {
                    return Err(VirtualConstraintError::Unsatisfied {
                        package:package.name.clone(),spec:text.clone(),name:measured.name.as_normalized().into(),
                        version:measured.version.to_string(),build:measured.build_string.clone(),
                    });
                }
            }
        }
    }
    Ok(())
}
fn invalid_spec(package: &LockedCondaPackage,spec: &str,detail: String) -> VirtualConstraintError {
    VirtualConstraintError::Spec { package:package.name.clone(),spec:spec.into(),detail }
}
pub(crate) fn condition_holds(condition: &MatchSpecCondition,packages: &[RepoDataRecord],facts: &[GenericVirtualPackage]) -> bool {
    match condition {
        MatchSpecCondition::MatchSpec(spec) => {
            packages.iter().any(|record| spec.matches(record)) || facts.iter().any(|fact| spec.matches(fact))
        },
        MatchSpecCondition::And(left,right) => condition_holds(left,packages,facts) && condition_holds(right,packages,facts),
        MatchSpecCondition::Or(left,right) => condition_holds(left,packages,facts) || condition_holds(right,packages,facts),
    }
}

#[cfg(test)]
mod tests;
