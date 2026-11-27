use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::Result;
use crate::models::NutritionProfile;

pub fn load_profile(path: &Path) -> Result<NutritionProfile> {
    let raw = fs::read_to_string(path)?;
    let profile: NutritionProfile = serde_yaml::from_str(&raw)?;
    profile.validate()?;
    Ok(profile)
}

pub fn profile_version(profile: &NutritionProfile) -> Result<String> {
    let canonical = serde_json::to_vec(profile)?;
    Ok(format!("{:x}", Sha256::digest(canonical)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    #[test]
    fn example_loads_and_versions_stably() {
        let profile = load_profile(Path::new("config/profile.example.yaml")).unwrap();
        assert_eq!(
            profile_version(&profile).unwrap(),
            profile_version(&profile).unwrap()
        );
    }
    #[test]
    fn automatic_adjustment_is_rejected() {
        let yaml = "goal: x\nnutrition: {automatic_target_adjustment: true}\npreferences: {}\nplanning: {}\n";
        let profile: NutritionProfile = serde_yaml::from_str(yaml).unwrap();
        assert!(profile.validate().is_err());
    }
}
