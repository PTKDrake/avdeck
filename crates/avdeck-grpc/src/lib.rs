//! gRPC client for a running Android Emulator started with `-grpc <port>`.
//!
//! The generated modules mirror the proto packages so cross-package references resolve.

use tonic::metadata::{Ascii, MetadataValue};
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::{Channel, Endpoint};
use tonic::{Request, Status};

#[allow(clippy::all, clippy::pedantic)]
pub mod proto {
    pub mod android {
        pub mod emulation {
            pub mod control {
                tonic::include_proto!("android.emulation.control");
            }
        }
    }
    pub mod emulator_snapshot {
        tonic::include_proto!("emulator_snapshot");
    }
}

use control::emulator_controller_client::EmulatorControllerClient;
use control::snapshot_service_client::SnapshotServiceClient;
use control::ui_controller_client::UiControllerClient;
mod frames;

pub use frames::{Frame, SharedFrames};
pub use proto::android::emulation::control;

/// A full-resolution RGBA frame of a 2560x1600 tablet is ~16 MB, far above tonic's 4 MB default.
const MAX_MESSAGE_SIZE: usize = 256 * 1024 * 1024;

/// Adds `authorization: Bearer <token>` to every call. With `-grpc-use-token` the token is the
/// `grpc.token` of the emulator's discovery file (`avdeck_core::running`), not the console token.
#[derive(Clone)]
pub struct Auth(Option<MetadataValue<Ascii>>);

impl Auth {
    pub fn none() -> Self {
        Self(None)
    }

    pub fn bearer(token: &str) -> Result<Self, tonic::metadata::errors::InvalidMetadataValue> {
        Ok(Self(Some(format!("Bearer {token}").parse()?)))
    }
}

impl Interceptor for Auth {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        if let Some(value) = &self.0 {
            request
                .metadata_mut()
                .insert("authorization", value.clone());
        }
        Ok(request)
    }
}

type Service = InterceptedService<Channel, Auth>;

/// Clients for one emulator. Cloning is cheap; all clones share the HTTP/2 connection.
#[derive(Clone)]
pub struct Emulator {
    pub controller: EmulatorControllerClient<Service>,
    pub snapshots: SnapshotServiceClient<Service>,
    pub ui: UiControllerClient<Service>,
}

impl Emulator {
    pub async fn connect(port: u16, auth: Auth) -> Result<Self, tonic::transport::Error> {
        let channel = Endpoint::from_shared(format!("http://127.0.0.1:{port}"))?
            .connect()
            .await?;
        Ok(Self {
            controller: EmulatorControllerClient::with_interceptor(channel.clone(), auth.clone())
                .max_decoding_message_size(MAX_MESSAGE_SIZE),
            snapshots: SnapshotServiceClient::with_interceptor(channel.clone(), auth.clone())
                .max_decoding_message_size(MAX_MESSAGE_SIZE),
            ui: UiControllerClient::with_interceptor(channel, auth),
        })
    }
}
