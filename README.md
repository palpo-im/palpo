# Palpo: A High-Performance Matrix Homeserver

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.89%2B-orange.svg)](https://www.rust-lang.org)

Palpo is a next-generation Matrix homeserver implementation written in Rust, designed for high performance, scalability, and robust federation. Built on the Salvo web framework and PostgreSQL database, Palpo delivers enterprise-grade real-time messaging and collaboration while maintaining minimal operational overhead.

---

## 🌟 What is Matrix?

Matrix is an open standard for decentralized, real-time communication. It enables:
- **End-to-end encrypted messaging** - Secure conversations by default
- **Federated architecture** - No single point of control or failure
- **Interoperability** - Bridge with other chat platforms (Slack, Discord, IRC, etc.)
- **VoIP and Video Calling** - Built-in support for voice and video
- **Rich messaging** - Reactions, threads, file sharing, and more

Learn more at [matrix.org](https://matrix.org)

---

## ✨ Why Palpo?

### **Performance First**
- Built with **Rust** for memory safety and zero-cost abstractions
- Powered by **Salvo** web framework for high-throughput async I/O
- **PostgreSQL** backend for reliable data persistence and ACID compliance
- Optimized for low latency and high concurrency

### **Developer Friendly**
- Clean, modular codebase
- Well-documented APIs
- Docker-ready deployment

### **Resource Efficient**
- Minimal memory footprint compared to reference implementations
- Efficient database query patterns
- Smart caching strategies
- Scales horizontally

---

## 🚀 Quick Start

### Try Our Demo Server

**⚠️ IMPORTANT: Test Server Notice**

Our test server is for **evaluation and testing purposes only**:
- **URL**: `https://test.palpo.im`
- **⚠️ All data on this server will be periodically deleted without notice**
- **Do NOT use for production or store important conversations**
- **Expected to be reset frequently for testing purposes**

#### Connect with Cinny (Web Client)
1. Open [Cinny](https://app.cinny.in/)
2. Click "Edit" on the homeserver selection
3. Enter `https://test.palpo.im` as your custom homeserver
4. Create a test account and start chatting!

#### Connect with Element (Desktop/Mobile)
1. Download [Element](https://element.io/download)
2. On login screen, click "Edit" next to the homeserver
3. Enter `https://test.palpo.im`
4. Register or login

---

## 📦 Installation

### Prerequisites
- Rust 1.89 or higher
- PostgreSQL 16 or higher
- Linux, macOS, or Windows (WSL2 recommended)

### Build from Source

```bash
# Clone the repository
git clone https://github.com/palpo-im/palpo.git
cd palpo

# Build the project
cargo build --release

# Copy example configuration
cp palpo-example.toml palpo.toml

# Edit configuration (set your domain, database credentials, etc.)
nano palpo.toml

# Run the server
./target/release/palpo
```

### Docker Deployment

```bash
# Pull the image
docker pull ghcr.io/palpo-im/palpo:latest

# Run with docker-compose
cd deploy/docker
docker-compose up -d
```

See [installation](https://palpo.im/guide/installation/) for detailed deployment instructions.

---

## 🧪 Current Progress

We use [Complement](https://github.com/matrix-org/complement) for comprehensive end-to-end testing against the Matrix specification.

- **Test Results**: [test_all.result.jsonl](tests/results/test_all.result.jsonl)
- **Test Coverage**: Continuously improving compliance with Matrix spec
- **Federation**: Active testing with other homeserver implementations

---

## 🤝 Contributing

We welcome contributions of all kinds! Whether you're:
- 🐛 Fixing bugs
- ✨ Adding features
- 📝 Improving documentation
- 🧪 Writing tests
- 🎨 Improving UX

Please read our [Contributing Guide](CONTRIBUTING.md) to get started.

---

---

## 🙏 Acknowledgments

Palpo stands on the shoulders of giants. We're grateful to these projects:

- **[Conduit](https://gitlab.com/famedly/conduit)** - Pioneering lightweight Matrix homeserver in Rust
- **[Ruma](https://github.com/ruma/ruma)** - Essential Matrix types and protocol implementations
- **[Tuwunel](https://github.com/matrix-construct/tuwunel)** - Innovative Matrix server architecture insights
- **[Salvo](https://github.com/salvo-rs/salvo)** - High-performance async web framework
- **[Matrix.org](https://matrix.org)** - For creating and maintaining the Matrix protocol

---

## 📄 License

Palpo is licensed under the Apache License 2.0. See [LICENSE](LICENSE) for details.

---

## 🔗 Links

- **Website**: [https://palpo.im](https://palpo.im)
- **Source Code**: [https://github.com/palpo-im/palpo](https://github.com/palpo-im/palpo)
- **Issue Tracker**: [https://github.com/palpo-im/palpo/issues](https://github.com/palpo-im/palpo/issues)
- **Demo Server**: [https://test.palpo.im](https://test.palpo.im) ⚠️ Test data will be deleted

---

## ⚠️ Important Notices

### Test Server Data Retention
**The test server at `test.palpo.im` is for evaluation only.** 
- All accounts, rooms, and messages may be deleted at any time
- Data is not backed up
- Service may be interrupted for updates
- **Do not rely on this server for any important communications**

### Production Use
Palpo is under active development. While we strive for stability:
- Always backup your database
- Test updates in a staging environment
- Monitor server logs and performance

---

**Built with ❤️ by the Palpo community**
