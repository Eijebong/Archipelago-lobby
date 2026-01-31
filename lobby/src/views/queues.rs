use crate::jobs::{
    GenerationParams, OptionsGenParams, OptionsGenResponse, YamlValidationParams,
    YamlValidationResponse,
};

wq::declare_queues!(
    yaml_validation<YamlValidationParams, YamlValidationResponse>,
    generation<GenerationParams, ()>,
    options_gen<OptionsGenParams, OptionsGenResponse>
);
