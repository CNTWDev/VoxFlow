pub mod backend;
pub mod cloud;
pub mod voxnexus;
mod wav;
pub mod error;

pub use backend::{
    provider_descriptors, AsrBackend, AsrCapabilities, AsrProviderDescriptor, AsrTransportKind,
    AudioChunk, StreamEvent, StreamTranscribeRequest, StreamingTranscriber, TranscribeRequest,
    TranscribeResult,
};
pub use cloud::{CloudBackend, OpenAiBackend};
pub use voxnexus::VoxNexusBackend;
pub use error::AsrError;
