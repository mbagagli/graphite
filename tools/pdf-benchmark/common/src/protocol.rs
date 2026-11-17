use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};

use crate::model::{BenchmarkExtractionEnvelopeV1, CandidateDescriptor, ExtractionRequest};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ProtocolRequest {
    Handshake { protocol_version: u32 },
    Extract { request: Box<ExtractionRequest> },
    Shutdown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProtocolResponse {
    Handshake {
        protocol_version: u32,
        candidate: CandidateDescriptor,
    },
    Extraction {
        result: Box<BenchmarkExtractionEnvelopeV1>,
    },
    Error {
        message: String,
    },
    Shutdown,
}

pub trait Adapter {
    fn descriptor(&self) -> CandidateDescriptor;

    fn extract(&mut self, request: ExtractionRequest) -> BenchmarkExtractionEnvelopeV1;
}

pub fn run_adapter(adapter: &mut dyn Adapter) -> Result<(), String> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|error| error.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let request = match serde_json::from_str::<ProtocolRequest>(&line) {
            Ok(request) => request,
            Err(error) => {
                write_response(
                    &mut stdout,
                    &ProtocolResponse::Error {
                        message: format!("invalid protocol request: {error}"),
                    },
                )?;
                continue;
            }
        };
        let response = match request {
            ProtocolRequest::Handshake { protocol_version } if protocol_version == 1 => {
                ProtocolResponse::Handshake {
                    protocol_version,
                    candidate: adapter.descriptor(),
                }
            }
            ProtocolRequest::Handshake { protocol_version } => ProtocolResponse::Error {
                message: format!("unsupported protocol version {protocol_version}"),
            },
            ProtocolRequest::Extract { request } => ProtocolResponse::Extraction {
                result: Box::new(adapter.extract(*request)),
            },
            ProtocolRequest::Shutdown => {
                write_response(&mut stdout, &ProtocolResponse::Shutdown)?;
                return Ok(());
            }
        };
        write_response(&mut stdout, &response)?;
    }
    Ok(())
}

fn write_response(output: &mut impl Write, response: &ProtocolResponse) -> Result<(), String> {
    serde_json::to_writer(&mut *output, response).map_err(|error| error.to_string())?;
    output.write_all(b"\n").map_err(|error| error.to_string())?;
    output.flush().map_err(|error| error.to_string())
}
