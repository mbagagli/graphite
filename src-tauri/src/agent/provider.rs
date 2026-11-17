use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::error::AppResult;

use super::model::{GenerationRequest, GenerationResult};

pub const ESTIMATED_TOKEN_ACCOUNTING: &str = "estimated-characters-3.0-v1";

pub trait EvidenceTokenCounter {
    fn count_tokens(&self, text: &str) -> AppResult<u32>;

    fn token_accounting_method(&self) -> &'static str;
}

pub trait LocalModelProvider: EvidenceTokenCounter + Send + Sync {
    fn provider_id(&self) -> &'static str;

    fn provider_label(&self) -> String;

    fn model_label(&self) -> String;

    fn context_capacity(&self) -> u32;

    fn generate(
        &self,
        request: &GenerationRequest,
        cancellation: Arc<AtomicBool>,
        on_delta: &mut dyn FnMut(&str),
    ) -> AppResult<GenerationResult>;
}

pub struct ConservativeTokenEstimator;

impl EvidenceTokenCounter for ConservativeTokenEstimator {
    fn count_tokens(&self, text: &str) -> AppResult<u32> {
        Ok(((text.chars().count() as f64 / 3.0).ceil() as u32).max(1))
    }

    fn token_accounting_method(&self) -> &'static str {
        ESTIMATED_TOKEN_ACCOUNTING
    }
}

#[cfg(test)]
pub struct DeterministicFakeProvider {
    pub response: String,
}

#[cfg(test)]
impl EvidenceTokenCounter for DeterministicFakeProvider {
    fn count_tokens(&self, text: &str) -> AppResult<u32> {
        Ok(text.split_whitespace().count() as u32)
    }

    fn token_accounting_method(&self) -> &'static str {
        "deterministic-whitespace-test-v1"
    }
}

#[cfg(test)]
impl LocalModelProvider for DeterministicFakeProvider {
    fn provider_id(&self) -> &'static str {
        "deterministic-fake"
    }

    fn provider_label(&self) -> String {
        "Deterministic test provider".to_owned()
    }

    fn model_label(&self) -> String {
        "M5 fixture model".to_owned()
    }

    fn context_capacity(&self) -> u32 {
        8_192
    }

    fn generate(
        &self,
        _request: &GenerationRequest,
        cancellation: Arc<AtomicBool>,
        on_delta: &mut dyn FnMut(&str),
    ) -> AppResult<GenerationResult> {
        if cancellation.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(crate::error::AppError::Agent(
                "Generation was cancelled before it began.".to_owned(),
            ));
        }
        on_delta(&self.response);
        Ok(GenerationResult {
            text: self.response.clone(),
            generated_tokens: self.response.split_whitespace().count() as u32,
            time_to_first_token_ms: Some(1),
            generation_ms: 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use super::{
        ConservativeTokenEstimator, DeterministicFakeProvider, EvidenceTokenCounter,
        LocalModelProvider,
    };
    use crate::agent::model::GenerationRequest;

    fn request() -> GenerationRequest {
        GenerationRequest {
            system_prompt: "Ground answers".to_owned(),
            user_prompt: "Question".to_owned(),
            max_tokens: 32,
            context_tokens: 8_192,
            temperature: 0.0,
        }
    }

    #[test]
    fn fake_provider_streams_a_deterministic_cited_answer() {
        let provider = DeterministicFakeProvider {
            response: "Supported finding [S1].".to_owned(),
        };
        let mut streamed = String::new();
        let result = provider
            .generate(&request(), Arc::new(AtomicBool::new(false)), &mut |delta| {
                streamed.push_str(delta)
            })
            .expect("fake generation");
        assert_eq!(streamed, provider.response);
        assert_eq!(result.text, provider.response);
    }

    #[test]
    fn fake_provider_obeys_pre_generation_cancellation() {
        let provider = DeterministicFakeProvider {
            response: "Answer".to_owned(),
        };
        let cancelled = Arc::new(AtomicBool::new(true));
        assert!(provider
            .generate(&request(), cancelled, &mut |_| {})
            .is_err());
    }

    #[test]
    fn deterministic_and_external_counters_report_truthful_methods() {
        let fake = DeterministicFakeProvider {
            response: String::new(),
        };
        assert_eq!(fake.count_tokens("one two three").expect("test tokens"), 3);
        assert_eq!(
            fake.token_accounting_method(),
            "deterministic-whitespace-test-v1"
        );
        assert_eq!(
            ConservativeTokenEstimator.token_accounting_method(),
            "estimated-characters-3.0-v1"
        );
        assert_eq!(
            ConservativeTokenEstimator
                .count_tokens("1234567")
                .expect("estimated tokens"),
            3
        );
    }
}
