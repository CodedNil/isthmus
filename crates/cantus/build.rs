use protobuf::{
    descriptor::field_descriptor_proto::Type,
    reflect::{FieldDescriptor, MessageDescriptor},
};
use protobuf_codegen::{Customize, CustomizeCallback};
use std::{env, path::PathBuf};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        generate_spotify_protocol();
    }
    isthmus_build::build("src/render/mod.rs").expect("failed to build shaders");
}

// The checked-in Spotify schemas are the wire contract. Only the Rust bindings are generated.
fn generate_spotify_protocol() {
    let proto_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("src/music/spotify/proto");
    println!("cargo:rerun-if-changed=src/music/spotify/proto");

    let files = [
        "collection.proto",
        "connect.proto",
        "extended_metadata.proto",
        "list_read.proto",
        "metadata.proto",
        "playlist4_external.proto",
    ]
    .map(|file| proto_dir.join(file));

    protobuf_codegen::Codegen::new()
        .pure()
        .cargo_out_dir("spotify")
        .customize_callback(DealerJson)
        .inputs(&files)
        .include(&proto_dir)
        .run()
        .expect("Codegen failed.");
}

// The Dealer occasionally puts a ProtoJSON cluster in the JSON transport envelope.
struct DealerJson;

impl DealerJson {
    fn supports(message: &MessageDescriptor) -> bool {
        matches!(
            message.full_name(),
            "spotify.connectstate.ClusterUpdate"
                | "spotify.connectstate.Cluster"
                | "spotify.connectstate.PlayerState"
                | "spotify.connectstate.ProvidedTrack"
        )
    }
}

impl CustomizeCallback for DealerJson {
    fn message(&self, message: &MessageDescriptor) -> Customize {
        if Self::supports(message) {
            Customize::default().before("#[derive(serde::Deserialize)]\n#[serde(default, rename_all = \"camelCase\")]")
        } else {
            Customize::default()
        }
    }

    fn field(&self, field: &FieldDescriptor) -> Customize {
        if !Self::supports(&field.containing_message()) {
            return Customize::default();
        }
        let name = field.name();
        if matches!(name, "options" | "suppressions") {
            return Customize::default().before("#[serde(skip)]");
        }
        let kind = field.proto().type_();
        let decoder = if matches!(kind, Type::TYPE_INT64 | Type::TYPE_DOUBLE) {
            "number"
        } else if kind == Type::TYPE_MESSAGE && field.is_singular() {
            "message"
        } else {
            "default_on_null"
        };
        let alias = if name.contains('_') { format!("alias = {name:?}, ") } else { String::new() };
        Customize::default().before(&format!("#[serde({alias}deserialize_with = \"super::super::dealer::{decoder}\")]"))
    }

    fn special_field(&self, message: &MessageDescriptor, _field: &str) -> Customize {
        if Self::supports(message) { Customize::default().before("#[serde(skip)]") } else { Customize::default() }
    }
}
