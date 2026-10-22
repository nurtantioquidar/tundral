//! Hosted Dijkstral (`../DIJKSTRAL_HOSTED_DESIGN.md`): GitHub webhooks, graph builds on push,
//! per-repository MCP over HTTP, and the web app's private API.

pub mod app;
pub mod builder;
pub mod config;
pub mod db;
pub mod git;
pub mod webhook;
