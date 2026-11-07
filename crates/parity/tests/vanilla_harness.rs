//! Manual smoke test of the vanilla server harness.
//!
//! Needs the owner's environment: an official `server.jar`, `eula=true`, a
//! configured `server.properties` and a world template (all set up by the
//! M3 environment step, see docs/PROGRESS.md).

#[tokio::test]
#[ignore = "requires the owner's vanilla installation (C:\\MineRust-data)"]
async fn vanilla_server_starts_and_stops() {
    let env = match parity::load_parity_env() {
        Ok(env) => env,
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            return;
        }
    };

    let vanilla = parity::Vanilla::start(&env)
        .await
        .expect("vanilla server must start");
    vanilla
        .stop()
        .await
        .expect("vanilla server must stop cleanly");
}
