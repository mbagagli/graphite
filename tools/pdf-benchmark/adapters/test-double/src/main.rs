use std::time::Duration;

use graphite_pdf_benchmark_common::{
    canonical_unstructured, run_adapter, Adapter, BenchmarkExtractionEnvelopeV1, BenchmarkProfile,
    CandidateDescriptor, ExtractionObservations, ExtractionOutcome, ExtractionRequest,
    TimingObservation, BENCHMARK_SCHEMA_VERSION,
};

fn main() {
    if let Err(error) = run_adapter(&mut TestDoubleAdapter) {
        eprintln!("benchmark protocol test double: {error}");
        std::process::exit(1);
    }
}

struct TestDoubleAdapter;

impl Adapter for TestDoubleAdapter {
    fn descriptor(&self) -> CandidateDescriptor {
        CandidateDescriptor {
            candidate_id: "test-double".to_owned(),
            version: "1".to_owned(),
            revision: "local-contract-fixture".to_owned(),
            license: "MIT".to_owned(),
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
            features: vec!["protocol-failure-injection".to_owned()],
            supported_profiles: vec![BenchmarkProfile::Native],
            configured_available: true,
            availability_reason: None,
        }
    }

    fn extract(&mut self, request: ExtractionRequest) -> BenchmarkExtractionEnvelopeV1 {
        match request.document_id.as_str() {
            "timeout" => {
                std::thread::sleep(Duration::from_secs(5));
                output(
                    &request,
                    self.descriptor(),
                    ExtractionOutcome::Succeeded,
                    "late",
                )
            }
            "crash" => std::process::exit(86),
            "unsupported" => BenchmarkExtractionEnvelopeV1::terminal(
                &request,
                self.descriptor(),
                ExtractionOutcome::Unsupported,
                "synthetic media/profile mismatch",
            ),
            "unavailable" => BenchmarkExtractionEnvelopeV1::terminal(
                &request,
                self.descriptor(),
                ExtractionOutcome::Unavailable,
                "synthetic model asset is unavailable",
            ),
            "failed" => BenchmarkExtractionEnvelopeV1::terminal(
                &request,
                self.descriptor(),
                ExtractionOutcome::Failed,
                "synthetic parser error",
            ),
            "empty-success" => output(
                &request,
                self.descriptor(),
                ExtractionOutcome::EmptySuccess,
                "",
            ),
            "partial" => output(
                &request,
                self.descriptor(),
                ExtractionOutcome::Partial,
                "partial searchable output",
            ),
            _ => output(
                &request,
                self.descriptor(),
                ExtractionOutcome::Succeeded,
                "complete searchable output",
            ),
        }
    }
}

fn output(
    request: &ExtractionRequest,
    candidate: CandidateDescriptor,
    outcome: ExtractionOutcome,
    text: &str,
) -> BenchmarkExtractionEnvelopeV1 {
    let canonical = canonical_unstructured(
        &request.artifact_id,
        request.artifact_revision_id,
        "test-double:1:native",
        text,
        None,
    );
    BenchmarkExtractionEnvelopeV1 {
        schema_version: BENCHMARK_SCHEMA_VERSION.to_owned(),
        request_id: request.request_id.clone(),
        document_id: request.document_id.clone(),
        input_sha256: request.input_sha256.clone(),
        candidate,
        profile: request.profile.clone(),
        outcome,
        outcome_reason: None,
        searchable_projection: Some(text.to_owned()),
        document: None,
        canonical_document: Some(canonical),
        mapping_losses: Vec::new(),
        warnings: Vec::new(),
        observations: ExtractionObservations::default(),
        timing: TimingObservation::default(),
        diagnostics: Default::default(),
    }
}
