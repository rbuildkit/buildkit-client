//! Integration tests requiring a running BuildKit daemon
//!
//! These tests will be skipped if BuildKit is not available.
//! Set BUILDKIT_ADDR environment variable to specify BuildKit address.
//! Default: http://localhost:1234
//!
//! Run with: cargo test --test integration_test -- --test-threads=1

mod common;

use buildkit_client::{BuildKitClient, BuildConfig};
use common::*;

#[tokio::test]
async fn test_buildkit_connection() {
    skip_without_buildkit!();

    let addr = get_buildkit_addr();
    let result = BuildKitClient::connect(&addr).await;

    assert!(result.is_ok(), "Failed to connect to BuildKit at {}", addr);
}

#[tokio::test]
async fn test_buildkit_health_check() {
    skip_without_buildkit!();

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let result = client.health_check().await;
    assert!(result.is_ok(), "Health check failed");
}

#[tokio::test]
async fn test_simple_local_build() {
    skip_without_buildkit!();

    let test_dir = create_temp_dir("simple-build");
    create_test_dockerfile(&test_dir, None);

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::local(&test_dir)
        .tag(random_test_tag());

    let result = client.build(config, None).await;

    cleanup_temp_dir(&test_dir);

    assert!(result.is_ok(), "Build failed: {:?}", result.err());

    let build_result = result.unwrap();
    println!("Build digest: {:?}", build_result.digest);
}

#[tokio::test]
async fn test_build_with_custom_dockerfile() {
    skip_without_buildkit!();

    let test_dir = create_temp_dir("custom-dockerfile");

    // Create a custom Dockerfile with a different name
    let custom_dockerfile = test_dir.join("Custom.Dockerfile");
    std::fs::write(&custom_dockerfile, r#"FROM alpine:latest
RUN echo "Custom Dockerfile"
"#).unwrap();

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::local(&test_dir)
        .dockerfile("Custom.Dockerfile")
        .tag(random_test_tag());

    let result = client.build(config, None).await;

    cleanup_temp_dir(&test_dir);

    assert!(result.is_ok(), "Build with custom Dockerfile failed: {:?}", result.err());
}

#[tokio::test]
async fn test_build_with_args() {
    skip_without_buildkit!();

    let test_dir = create_temp_dir("build-args");
    create_dockerfile_with_args(&test_dir);

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::local(&test_dir)
        .build_arg("VERSION", "1.2.3")
        .build_arg("BUILD_DATE", "2024-01-01")
        .tag(random_test_tag());

    let result = client.build(config, None).await;

    cleanup_temp_dir(&test_dir);

    assert!(result.is_ok(), "Build with args failed: {:?}", result.err());
}

#[tokio::test]
async fn test_multistage_build_with_target() {
    skip_without_buildkit!();

    let test_dir = create_temp_dir("multistage");
    create_multistage_dockerfile(&test_dir);

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    // Build only the builder stage
    let config = BuildConfig::local(&test_dir)
        .target("builder")
        .tag(random_test_tag());

    let result = client.build(config, None).await;

    cleanup_temp_dir(&test_dir);

    assert!(result.is_ok(), "Multistage build with target failed: {:?}", result.err());
}

#[tokio::test]
async fn test_build_with_context_files() {
    skip_without_buildkit!();

    let test_dir = create_temp_dir("context-files");
    create_test_context(&test_dir);

    // Update Dockerfile to use context files
    let dockerfile_content = r#"FROM alpine:latest
COPY app/main.txt /main.txt
COPY app/config.txt /config.txt
COPY app/subdir/data.txt /data.txt
RUN cat /main.txt /config.txt /data.txt
"#;
    std::fs::write(test_dir.join("Dockerfile"), dockerfile_content).unwrap();

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::local(&test_dir)
        .tag(random_test_tag());

    let result = client.build(config, None).await;

    cleanup_temp_dir(&test_dir);

    assert!(result.is_ok(), "Build with context files failed: {:?}", result.err());
}

#[tokio::test]
async fn test_build_with_no_cache() {
    skip_without_buildkit!();

    let test_dir = create_temp_dir("no-cache");
    create_test_dockerfile(&test_dir, None);

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::local(&test_dir)
        .no_cache(true)
        .tag(random_test_tag());

    let result = client.build(config, None).await;

    cleanup_temp_dir(&test_dir);

    assert!(result.is_ok(), "Build with no-cache failed: {:?}", result.err());
}

#[tokio::test]
async fn test_build_with_pull() {
    skip_without_buildkit!();

    let test_dir = create_temp_dir("pull");
    create_test_dockerfile(&test_dir, Some(r#"FROM alpine:latest
RUN apk add --no-cache curl
"#));

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::local(&test_dir)
        .pull(true)
        .tag(random_test_tag());

    let result = client.build(config, None).await;

    cleanup_temp_dir(&test_dir);

    assert!(result.is_ok(), "Build with pull failed: {:?}", result.err());
}

#[tokio::test]
async fn test_build_with_progress_handler() {
    skip_without_buildkit!();

    use buildkit_client::progress::ConsoleProgressHandler;

    let test_dir = create_temp_dir("progress");
    create_test_dockerfile(&test_dir, None);

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let progress = Box::new(ConsoleProgressHandler::new(true));

    let config = BuildConfig::local(&test_dir)
        .tag(random_test_tag());

    let result = client.build(config, Some(progress)).await;

    cleanup_temp_dir(&test_dir);

    assert!(result.is_ok(), "Build with progress handler failed: {:?}", result.err());
}

#[tokio::test]
async fn test_build_with_dockerignore() {
    skip_without_buildkit!();

    let test_dir = create_temp_dir("dockerignore");
    create_test_context(&test_dir);
    create_dockerignore(&test_dir, &["app/subdir/"]);

    // Dockerfile that will fail if subdir is copied
    let dockerfile_content = r#"FROM alpine:latest
COPY app /app
RUN test ! -d /app/subdir || (echo "subdir should be ignored" && exit 1)
"#;
    std::fs::write(test_dir.join("Dockerfile"), dockerfile_content).unwrap();

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::local(&test_dir)
        .tag(random_test_tag());

    let result = client.build(config, None).await;

    cleanup_temp_dir(&test_dir);

    assert!(result.is_ok(), "Build with .dockerignore failed: {:?}", result.err());
}

#[tokio::test]
async fn test_invalid_dockerfile_syntax() {
    skip_without_buildkit!();

    let test_dir = create_temp_dir("invalid-syntax");

    // Create an invalid Dockerfile
    std::fs::write(test_dir.join("Dockerfile"), "INVALID DOCKERFILE SYNTAX").unwrap();

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::local(&test_dir)
        .tag(random_test_tag());

    let result = client.build(config, None).await;

    cleanup_temp_dir(&test_dir);

    // Should fail with an error
    assert!(result.is_err(), "Build should fail with invalid Dockerfile syntax");
}

#[tokio::test]
async fn test_build_nonexistent_base_image() {
    skip_without_buildkit!();

    let test_dir = create_temp_dir("nonexistent-image");

    std::fs::write(test_dir.join("Dockerfile"),
        "FROM nonexistent-image-that-does-not-exist:latest\n").unwrap();

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::local(&test_dir)
        .tag(random_test_tag());

    let result = client.build(config, None).await;

    cleanup_temp_dir(&test_dir);

    // Should fail because the base image doesn't exist
    assert!(result.is_err(), "Build should fail with nonexistent base image");
}

#[tokio::test]
async fn test_multiple_tags() {
    skip_without_buildkit!();

    let test_dir = create_temp_dir("multiple-tags");
    create_test_dockerfile(&test_dir, None);

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::local(&test_dir)
        .tag(random_test_tag())
        .tag(random_test_tag())
        .tag(random_test_tag());

    let result = client.build(config, None).await;

    cleanup_temp_dir(&test_dir);

    assert!(result.is_ok(), "Build with multiple tags failed: {:?}", result.err());
}

#[tokio::test]
#[ignore] // This test is slow, run with --ignored
async fn test_large_context() {
    skip_without_buildkit!();

    let test_dir = create_temp_dir("large-context");
    create_test_dockerfile(&test_dir, None);

    // Create many files to test large context transfer
    let data_dir = test_dir.join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    for i in 0..100 {
        let file_path = data_dir.join(format!("file_{}.txt", i));
        std::fs::write(&file_path, format!("Content for file {}", i)).unwrap();
    }

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::local(&test_dir)
        .tag(random_test_tag());

    let result = client.build(config, None).await;

    cleanup_temp_dir(&test_dir);

    assert!(result.is_ok(), "Build with large context failed: {:?}", result.err());
}

// ============================================================================
// GitHub Repository Tests
// ============================================================================

#[tokio::test]
async fn test_github_public_repo_build() {
    skip_without_buildkit!();

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::github("https://github.com/buildkit-rs/hello-world-public");

    let result = client.build(config, None).await;

    assert!(result.is_ok(), "Build from public GitHub repo failed: {:?}", result.err());
}

#[tokio::test]
async fn test_github_public_repo_with_ref() {
    skip_without_buildkit!();

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::github("https://github.com/buildkit-rs/hello-world-public")
        .git_ref("main");

    let result = client.build(config, None).await;

    assert!(result.is_ok(), "Build from public GitHub repo with ref failed: {:?}", result.err());
}

#[tokio::test]
async fn test_github_private_repo_build() {
    skip_without_buildkit!();

    // Get GitHub token from environment
    let github_token = std::env::var("GITHUB_TOKEN")
        .unwrap_or_else(|_| "ffffff".to_string());

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::github("https://github.com/buildkit-rs/hello-world-private")
        .github_token(github_token);

    let result = client.build(config, None).await;

    assert!(result.is_ok(), "Build from private GitHub repo failed: {:?}", result.err());
}

#[tokio::test]
async fn test_github_private_repo_with_ref() {
    skip_without_buildkit!();

    let github_token = std::env::var("GITHUB_TOKEN")
        .unwrap_or_else(|_| "ffffff".to_string());

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::github("https://github.com/buildkit-rs/hello-world-private")
        .git_ref("main")
        .github_token(github_token);

    let result = client.build(config, None).await;

    assert!(result.is_ok(), "Build from private GitHub repo with ref failed: {:?}", result.err());
}

#[tokio::test]
async fn test_github_with_custom_dockerfile() {
    skip_without_buildkit!();

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::github("https://github.com/buildkit-rs/hello-world-public")
        .dockerfile("Dockerfile");

    let result = client.build(config, None).await;

    assert!(result.is_ok(), "Build from GitHub with custom Dockerfile failed: {:?}", result.err());
}

#[tokio::test]
async fn test_github_with_build_args() {
    skip_without_buildkit!();

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let config = BuildConfig::github("https://github.com/buildkit-rs/hello-world-public")
        .build_arg("VERSION", "1.0.0")
        .build_arg("BUILD_DATE", "2024-01-01");

    let result = client.build(config, None).await;

    assert!(result.is_ok(), "Build from GitHub with build args failed: {:?}", result.err());
}

#[tokio::test]
async fn test_github_private_without_token() {
    skip_without_buildkit!();

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    // Try to build private repo without token - should fail
    let config = BuildConfig::github("https://github.com/buildkit-rs/hello-world-private");

    let result = client.build(config, None).await;

    // This should fail because no authentication is provided
    assert!(result.is_err(), "Build from private GitHub repo without token should fail");
}

#[tokio::test]
async fn test_github_with_progress_handler() {
    skip_without_buildkit!();

    use buildkit_client::progress::ConsoleProgressHandler;

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    let progress = Box::new(ConsoleProgressHandler::new(true));

    let config = BuildConfig::github("https://github.com/buildkit-rs/hello-world-public");

    let result = client.build(config, Some(progress)).await;

    assert!(result.is_ok(), "Build from GitHub with progress handler failed: {:?}", result.err());
}

#[tokio::test]
#[ignore] // Requires valid commit hash
async fn test_github_with_commit_ref() {
    skip_without_buildkit!();

    let addr = get_buildkit_addr();
    let mut client = BuildKitClient::connect(&addr).await.unwrap();

    // Use a specific commit hash (this would need to be a real commit in the repo)
    let config = BuildConfig::github("https://github.com/buildkit-rs/hello-world-public")
        .git_ref("HEAD");

    let result = client.build(config, None).await;

    assert!(result.is_ok(), "Build from GitHub with commit ref failed: {:?}", result.err());
}
