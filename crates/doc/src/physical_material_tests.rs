use super::*;
use serde_json::json;

fn card(data: Value) -> Value {
    json!({"schema_version":1,"designation":"Original synthetic fixture","family":"wood",
        "sources":[{"id":"test","title":"Analytic synthetic data, not a material specification","kind":"illustrative","license":"MIT OR Apache-2.0"}],
        "properties":[{"data":data,"source_id":"test"}]})
}
fn isotropic(e: f64, nu: f64) -> Value {
    json!({"kind":"elasticity","model":{"symmetry":"isotropic","young_modulus_pa":e,"poisson_ratio":nu}})
}
fn orthotropic(e: [f64; 3], nu: [f64; 3]) -> Value {
    json!({"kind":"elasticity","model":{"symmetry":"orthotropic","axes":"wood_lrt",
        "young_moduli_pa":e,"shear_moduli_pa":[4e9,4e9,4e9],"poisson_ratios":nu}})
}

#[test]
fn directional_isotropy_and_principal_axes_match_analytic_values() {
    let iso = admit_card(&card(isotropic(12e9, 0.25))).unwrap();
    let mut equivalent = orthotropic([12e9; 3], [0.25; 3]);
    equivalent["model"]["shear_moduli_pa"] = json!([4.8e9, 4.8e9, 4.8e9]);
    let equivalent = admit_card(&card(equivalent)).unwrap();
    for direction in [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [1., 2., 3.], [-1., 2., -3.], [f64::MAX; 3], [1e-320; 3]] {
        let a = iso.directional_modulus(direction).unwrap();
        let b = equivalent.directional_modulus(direction).unwrap();
        assert_eq!(a.young_modulus_pa, 12e9);
        assert!((b.young_modulus_pa / 12e9 - 1.).abs() < 1e-14);
        assert_eq!(a.axes, None);
        assert_eq!(b.axes, Some(MaterialAxes::WoodLrt));
        assert!((a.direction.iter().map(|x| x * x).sum::<f64>() - 1.).abs() < 1e-14);
    }
    let ortho = admit_card(&card(orthotropic([12e9, 3e9, 1e9], [0.2, 0.1, 0.02]))).unwrap();
    for (direction, e) in [([1., 0., 0.], 12e9), ([0., 1., 0.], 3e9), ([0., 0., 1.], 1e9)] {
        assert!((ortho.directional_modulus(direction).unwrap().young_modulus_pa / e - 1.).abs() < 1e-14);
    }
}

#[test]
fn directional_orthotropy_includes_shear_and_reciprocal_coupling() {
    let mut data = orthotropic([12e9, 3e9, 1e9], [0.2, 0.1, 0.02]);
    data["model"]["shear_moduli_pa"] = json!([2e9, 1e9, 0.5e9]);
    let material = admit_card(&card(data)).unwrap();
    // For n=(1,1,0)/sqrt(2), 1/E(n)=(S11+S22+2S12+1/G12)/4.
    let expected = 4. / (1. / 12e9 + 1. / 3e9 - 2. * 0.2 / 12e9 + 1. / 2e9);
    for d in [[1., 1., 0.], [-2., 2., 0.], [1e200, 1e200, 0.]] {
        let r = material.directional_modulus(d).unwrap();
        assert!((r.young_modulus_pa / expected - 1.).abs() < 1e-14);
        assert_eq!(r.validation.source_review_required, vec!["test"]);
        assert!(!r.validation.analysis_solver_available);
    }
    // Independent direct tensor contraction in a general non-principal direction.
    let [x, y, z] = [2. / 3., -1. / 3., 2. / 3.];
    let inverse = x * x * x * x / 12e9
        + y * y * y * y / 3e9
        + z * z * z * z / 1e9
        + x * x * y * y * (1. / 2e9 - 2. * 0.2 / 12e9)
        + y * y * z * z * (1. / 1e9 - 2. * 0.1 / 3e9)
        + z * z * x * x * (1. / 0.5e9 - 2. * 0.02 / 1e9);
    let actual = material.directional_modulus([2., -1., 2.]).unwrap();
    assert!((actual.young_modulus_pa * inverse - 1.).abs() < 1e-14);
}

#[test]
fn directional_queries_refuse_unknown_unstable_and_nonfinite_inputs() {
    let mut material = admit_card(&card(isotropic(12e9, 0.25))).unwrap();
    for d in [[0.; 3], [f64::NAN, 0., 1.], [0., f64::INFINITY, 0.]] {
        assert!(material.directional_modulus(d).is_err());
    }
    material.properties.clear();
    assert!(material.directional_modulus([1., 0., 0.]).is_err());
    material.properties.push(Property {
        data: PropertyData::Elasticity { model: Elasticity::Isotropic { young_modulus_pa: 12e9, poisson_ratio: 0.6 } },
        source_id: "test".into(),
        conditions: Conditions::default(),
    });
    assert!(material.directional_modulus([1., 0., 0.]).is_err());
}

#[test]
fn isotropic_moduli_match_closed_form_and_auxetic_values_are_valid() {
    let r = validate_card(&card(isotropic(12e9, 0.25))).unwrap();
    assert_eq!(r.isotropic_moduli, Some(IsotropicModuli { shear_pa: 4.8e9, bulk_pa: 8e9 }));
    let r = validate_card(&card(isotropic(12e9, -0.5))).unwrap();
    assert_eq!(r.isotropic_moduli, Some(IsotropicModuli { shear_pa: 12e9, bulk_pa: 2e9 }));
    for nu in [-2., -1., 0.5, 0.6] {
        assert!(validate_card(&card(isotropic(12e9, nu))).is_err());
    }
    for e in [0., -1., 1e16] {
        assert!(validate_card(&card(isotropic(e, 0.25))).is_err());
    }
    assert!(Elasticity::Isotropic { young_modulus_pa: f64::NAN, poisson_ratio: 0.25 }.validate().is_err());
    assert!(Elasticity::Isotropic { young_modulus_pa: 1e-309, poisson_ratio: 0.25 }.validate().is_err());
}

#[test]
fn orthotropy_uses_reciprocity_and_full_positive_definiteness() {
    // With equal E and nu=.25 this is the same normal compliance as isotropy.
    assert!(validate_card(&card(orthotropic([12e9; 3], [0.25; 3]))).is_ok());
    // A general orthotropic ratio may exceed .5: reciprocal nu21 is .08 here.
    assert!(validate_card(&card(orthotropic([10e9, 1e9, 1e9], [0.8, 0., 0.]))).is_ok());
    // Every 2x2 principal minor is positive; the full determinant is negative.
    assert!(validate_card(&card(orthotropic([12e9; 3], [0.6; 3]))).is_err());
    assert!(validate_card(&card(orthotropic([10e9, 1e9, 1e9], [4., 0., 0.]))).is_err());
    assert!(validate_card(&card(orthotropic([0., 1e9, 1e9], [0.; 3]))).is_err());
    let mut bad = card(orthotropic([12e9; 3], [0.25; 3]));
    bad["properties"][0]["data"]["model"]["shear_moduli_pa"] = json!([4e9, 0., 4e9]);
    assert!(validate_card(&bad).is_err());
    // Scaling all moduli changes units of compliance, not its stability verdict.
    assert!(validate_card(&card(orthotropic([12.; 3], [0.25; 3]))).is_ok());
    assert!(validate_card(&card(orthotropic([1e-300, 1e15, 1.], [0.; 3]))).is_ok());
    assert!(validate_card(&card(orthotropic([1e-309, 1., 1.], [0.; 3]))).is_err());
}

#[test]
fn conditions_missing_values_and_source_claims_remain_explicit() {
    let mut value = card(json!({"kind":"density","kg_m3":600.}));
    let r = validate_card(&value).unwrap();
    assert_eq!(r.unspecified_conditions, vec!["density"]);
    assert_eq!(r.source_review_required, vec!["test"]); // Even an explicit license is unverified.
    assert!(!r.analysis_solver_available && !r.rendering_properties_available);
    assert!(r.isotropic_moduli.is_none());
    value["properties"][0]["conditions"] = json!({"temperature_k":293.15,"moisture_dry_basis_kg_kg":1.5,"state":"synthetic wet specimen"});
    assert!(validate_card(&value).unwrap().unspecified_conditions.is_empty());
    value["family"] = json!("polymer");
    value["properties"][0]["conditions"]["moisture_dry_basis_kg_kg"] = json!(150.);
    value["properties"][0]["conditions"]["strain_rate_per_s"] = json!(1.);
    assert!(validate_card(&value).unwrap().unspecified_conditions.is_empty());
    value["properties"] = json!([]);
    let r = validate_card(&value).unwrap();
    assert!(r.property_kinds.is_empty() && r.isotropic_moduli.is_none());
    let parsed: MaterialCard = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::from_value::<MaterialCard>(serde_json::to_value(&parsed).unwrap()).unwrap(), parsed);
}

#[test]
fn missing_condition_fields_follow_family_policy_without_inventing_values() {
    for (family, extra) in
        [("wood", Some("moisture_dry_basis_kg_kg")), ("polymer", Some("strain_rate_per_s")), ("glass", None), ("metal", None), ("other", None)]
    {
        let mut value = card(json!({"kind":"density","kg_m3":600.}));
        value["family"] = json!(family);
        let mut expected = vec!["temperature_k", "state"];
        expected.extend(extra);
        let result = validate_card(&value).unwrap();
        assert_eq!(result.missing_condition_fields, BTreeMap::from([("density".into(), expected.iter().map(|s| (*s).into()).collect())]));
        assert_eq!(result.unspecified_conditions, vec!["density"]);

        // Explicit zero dry-basis moisture is known; no wet/dry conversion or default is inferred.
        value["properties"][0]["conditions"] =
            json!({"temperature_k":293.15,"state":"synthetic","moisture_dry_basis_kg_kg":0.,"strain_rate_per_s":1.});
        assert!(validate_card(&value).unwrap().missing_condition_fields.is_empty());
        for field in expected {
            let mut partial = value.clone();
            partial["properties"][0]["conditions"].as_object_mut().unwrap().remove(field);
            let result = validate_card(&partial).unwrap();
            assert_eq!(result.missing_condition_fields, BTreeMap::from([("density".into(), vec![field.into()])]));
            assert_eq!(result.unspecified_conditions, vec!["density"]);
        }
        value["properties"] = json!([]);
        let result = validate_card(&value).unwrap();
        assert!(result.missing_condition_fields.is_empty() && result.unspecified_conditions.is_empty());
    }
}

#[test]
fn missing_condition_fields_are_per_property_and_reports_remain_backward_readable() {
    let mut value = card(json!({"kind":"density","kg_m3":600.}));
    value["properties"][0]["conditions"] = json!({"temperature_k":293.15,"state":"synthetic"});
    value["properties"].as_array_mut().unwrap().push(json!({"data":isotropic(12e9,0.25),"source_id":"test",
        "conditions":{"state":"synthetic","moisture_dry_basis_kg_kg":1.5}}));
    let before = value.clone();
    let result = validate_card(&value).unwrap();
    let expected = json!({"density":["moisture_dry_basis_kg_kg"],"elasticity":["temperature_k"]});
    let serialized = serde_json::to_value(&result).unwrap();
    assert_eq!(serialized["missing_condition_fields"], expected);
    assert_eq!(result.unspecified_conditions, vec!["density", "elasticity"]);
    assert_eq!(value, before);
    assert_eq!(serde_json::from_value::<Validation>(serialized.clone()).unwrap(), result);
    let mut old = serialized;
    old.as_object_mut().unwrap().remove("missing_condition_fields");
    let old: Validation = serde_json::from_value(old).unwrap();
    assert!(old.missing_condition_fields.is_empty());
    assert_eq!(old.unspecified_conditions, result.unspecified_conditions);
    assert_eq!(old.source_review_required, result.source_review_required);
}

#[test]
fn thermal_properties_accept_negative_expansion_but_not_negative_conductivity() {
    assert!(validate_card(&card(json!({"kind":"thermal_expansion","per_k":[-1e-6,0.,2e-6],"axes":"material123"}))).is_ok());
    assert!(validate_card(&card(json!({"kind":"thermal_conductivity","w_m_k":[1.,0.,1.],"axes":"material123"}))).is_err());
    assert!(validate_card(&card(json!({"kind":"specific_heat","j_kg_k":500.}))).is_ok());
    for kind in ["tensile_strength", "compressive_strength", "yield_strength", "shear_strength"] {
        assert!(validate_card(&card(json!({"kind":kind,"pa":1e8}))).is_ok());
        assert!(validate_card(&card(json!({"kind":kind,"pa":-1.}))).is_err());
    }
}

#[test]
fn hostile_units_sources_fields_and_budgets_are_rejected() {
    let valid = card(isotropic(12e9, 0.25));
    let mut bad = valid.clone();
    bad["properties"][0]["data"]["model"]["young_modulus_mpa"] = json!(12.);
    assert!(validate_card(&bad).is_err());
    let mut bad = valid.clone();
    bad["properties"][0]["source_id"] = json!("unknown");
    assert!(validate_card(&bad).is_err());
    let mut bad = valid.clone();
    bad["properties"] = json!([valid["properties"][0], valid["properties"][0]]);
    assert!(validate_card(&bad).is_err());
    let mut bad = valid.clone();
    bad["sources"] = json!([valid["sources"][0], valid["sources"][0]]);
    assert!(validate_card(&bad).is_err());
    let mut bad = valid.clone();
    bad["schema_version"] = json!(2);
    assert!(validate_card(&bad).is_err());
    let mut bad = valid.clone();
    bad["designation"] = json!("x".repeat(257));
    assert!(validate_card(&bad).is_err());
    let mut bad = valid.clone();
    bad["properties"][0]["conditions"] = json!({"moisture_dry_basis_kg_kg":-1.});
    assert!(validate_card(&bad).is_err());
    assert!(validate_card(&json!(vec![Value::Null; 33])).is_err());
    assert!(validate_card(&json!("x".repeat(256 * 1024 + 1))).is_err());
    let mut deep = Value::Null;
    for _ in 0..14 {
        deep = json!([deep]);
    }
    assert!(validate_card(&deep).is_err());
}
