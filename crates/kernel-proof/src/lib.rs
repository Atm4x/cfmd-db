pub trait CertificateChecker {
    type Spec;
    type Certificate;
    type Error;

    fn check(spec: &Self::Spec, certificate: &Self::Certificate) -> Result<(), Self::Error>;
}

pub struct CheckedCertificate<C: CertificateChecker> {
    spec: C::Spec,
    certificate: C::Certificate,
    _checker: std::marker::PhantomData<fn() -> C>,
}

impl<C: CertificateChecker> CheckedCertificate<C> {
    #[must_use]
    pub const fn certificate(&self) -> &C::Certificate {
        &self.certificate
    }

    #[must_use]
    pub const fn spec(&self) -> &C::Spec {
        &self.spec
    }

    #[must_use]
    pub fn into_inner(self) -> C::Certificate {
        self.certificate
    }
}

pub fn verify_certificate<C: CertificateChecker>(
    spec: &C::Spec,
    certificate: C::Certificate,
) -> Result<CheckedCertificate<C>, C::Error>
where
    C::Spec: Clone,
{
    C::check(spec, &certificate)?;
    Ok(CheckedCertificate {
        spec: spec.clone(),
        certificate,
        _checker: std::marker::PhantomData,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cell {
    I64(i64),
    Bool(bool),
}

pub type Row = Vec<Cell>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Predicate {
    True,
    EqI64 { column: usize, value: i64 },
    And(Box<Self>, Box<Self>),
}

impl Predicate {
    #[must_use]
    pub fn evaluate(&self, row: &Row) -> bool {
        match self {
            Self::True => true,
            Self::EqI64 { column, value } => {
                matches!(row.get(*column), Some(Cell::I64(actual)) if actual == value)
            }
            Self::And(left, right) => left.evaluate(row) && right.evaluate(row),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    Input,
    Filter {
        predicate: Predicate,
        input: Box<Self>,
    },
}

impl Plan {
    #[must_use]
    pub fn execute(&self, input_rows: &[Row]) -> Vec<Row> {
        match self {
            Self::Input => input_rows.to_vec(),
            Self::Filter { predicate, input } => input
                .execute(input_rows)
                .into_iter()
                .filter(|row| predicate.evaluate(row))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteCertificate {
    FuseNestedFilters,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteSpec {
    pub before: Plan,
    pub after: Plan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofError {
    InvalidRewrite,
}

pub struct RewriteChecker;

impl CertificateChecker for RewriteChecker {
    type Spec = RewriteSpec;
    type Certificate = RewriteCertificate;
    type Error = ProofError;

    fn check(spec: &Self::Spec, certificate: &Self::Certificate) -> Result<(), Self::Error> {
        check_rewrite(&spec.before, &spec.after, *certificate)
            .then_some(())
            .ok_or(ProofError::InvalidRewrite)
    }
}

pub fn verify_rewrite(
    before: Plan,
    after: Plan,
    certificate: RewriteCertificate,
) -> Result<CheckedCertificate<RewriteChecker>, ProofError> {
    verify_certificate::<RewriteChecker>(&RewriteSpec { before, after }, certificate)
}

#[must_use]
pub fn check_rewrite(before: &Plan, after: &Plan, certificate: RewriteCertificate) -> bool {
    match certificate {
        RewriteCertificate::FuseNestedFilters => {
            let Plan::Filter {
                predicate: outer,
                input: outer_input,
            } = before
            else {
                return false;
            };
            let Plan::Filter {
                predicate: inner,
                input: base,
            } = outer_input.as_ref()
            else {
                return false;
            };
            let expected = Plan::Filter {
                predicate: Predicate::And(Box::new(inner.clone()), Box::new(outer.clone())),
                input: base.clone(),
            };
            *after == expected
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checker_accepts_only_the_exact_filter_fusion_rule() {
        let p = Predicate::EqI64 {
            column: 0,
            value: 1,
        };
        let q = Predicate::EqI64 {
            column: 1,
            value: 2,
        };
        let before = Plan::Filter {
            predicate: q.clone(),
            input: Box::new(Plan::Filter {
                predicate: p.clone(),
                input: Box::new(Plan::Input),
            }),
        };
        let after = Plan::Filter {
            predicate: Predicate::And(Box::new(p), Box::new(q)),
            input: Box::new(Plan::Input),
        };
        assert!(check_rewrite(
            &before,
            &after,
            RewriteCertificate::FuseNestedFilters
        ));

        let rows = vec![
            vec![Cell::I64(1), Cell::I64(2)],
            vec![Cell::I64(1), Cell::I64(3)],
            vec![Cell::I64(0), Cell::I64(2)],
        ];
        assert_eq!(before.execute(&rows), after.execute(&rows));
    }

    #[test]
    fn checker_rejects_wrong_rewrite_even_if_shape_is_similar() {
        let before = Plan::Filter {
            predicate: Predicate::True,
            input: Box::new(Plan::Filter {
                predicate: Predicate::True,
                input: Box::new(Plan::Input),
            }),
        };
        assert!(!check_rewrite(
            &before,
            &Plan::Input,
            RewriteCertificate::FuseNestedFilters
        ));
    }
    #[test]
    fn optimizer_rewrite_uses_the_same_checked_certificate_boundary() {
        let predicate = Predicate::EqI64 {
            column: 0,
            value: 7,
        };
        let before = Plan::Filter {
            predicate: predicate.clone(),
            input: Box::new(Plan::Filter {
                predicate: Predicate::True,
                input: Box::new(Plan::Input),
            }),
        };
        let after = Plan::Filter {
            predicate: Predicate::And(Box::new(Predicate::True), Box::new(predicate)),
            input: Box::new(Plan::Input),
        };
        let checked = verify_rewrite(before, after, RewriteCertificate::FuseNestedFilters).unwrap();
        assert_eq!(
            checked.certificate(),
            &RewriteCertificate::FuseNestedFilters
        );
        assert!(matches!(checked.spec().before, Plan::Filter { .. }));
    }
}
