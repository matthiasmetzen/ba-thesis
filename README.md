<div align="center">
  <img src="https://coconucos.cs.hhu.de/lehre/bigdata/resources/img/hhu-logo.svg" width=300>

  [![Download](https://img.shields.io/static/v1?label=&message=pdf&color=EE3F24&style=for-the-badge&logo=adobe-acrobat-reader&logoColor=FFFFFF)](/../-/jobs/artifacts/master/file/document/thesis.pdf?job=latex)
</div>

# :notebook: &nbsp; Aufgabenbeschreibung

This thesis focuses on the development and implementation of a lightweight and extensible proxy with caching functionality for S3 systems. The primary objective is to enhance access times for personal or company internal use, where it can run alongside a service requiring rapid and recurrent access that will profit from even small decreases in access times.

The implementation will primarily focus on the caching functionality of the proxy and its compatibility with the S3 API.
Furthermore, the implementation will emphasize the proxy's extensibility and configurability. The terms \textit{internal composability} and \textit{external composability} will be defined and used to describe the design approach:

\begin{description}
	\label{def:composition}
	\item[Internal composability] The implementation aims for a modular design in which parts of the software can easily be swapped, reconfigured or extended. The reconfiguration of modules will be done through simple configuration files.
	\item[External composability] To further reduce access times, the software will be enabled to be chained with other instances of itself, to allow for multiple levels of caching and to further extend the software's capabilities eg. by sharing data or by providing more efficient communication.
\end{description}

# Building

The project is build with Rust nightly

```sh
cd project/s3p
cargo build
```

# Running

```
> s3p --help
Arguments for loading configuration from file

Usage: s3p [OPTIONS]

Options:
  -c, --config-file <CONFIG_FILE>  
  -r, --regenerate                 
  -g, --generate-if-missing        
  -h, --help                       Print help
  -V, --version                    Print version
```
## Configuration

```toml
# Log Level
logLevel = "critical"

# Server configuration
[server]
# Server type. Available: [s3]
type = "s3"
# host address to listen on
host = "127.0.0.1"
# port to listen on
port = 4356
# validate credentails
validateCredentials = false

# (Optional) Server credentails for validation. Ignored if validateCredentials = false
# Required if validateCredentials = true
[server.credentials]
accessKeyId = "user"
secretKey = "password"

# List of middlewares
[[middlewares]]
# Middleware type. Avaliable [cache, identity]
type = "cache"
# Cache size in bytes. Requests without body count as size 1
cacheSize = 500_000_000
# (Optional) Global Time-To-Live in ms
ttl = 1000_000
# (Optional) Global Time-To-Idle in ms
tti = 1000_000

# GetObject config for middleware
[middlewares.ops.GetObject]
enabled = true
# (Optional) GetObject Time-To-Live in ms
# ttl = 1000_000
# (Optional) GetObject Time-To-Idle in ms
# tti = 1000_000

[middlewares.ops.HeadObject]
enabled = true

[middlewares.ops.ListObjects]
enabled = true

[middlewares.ops.ListObjectVersions]
enabled = true

[middlewares.ops.HeadBucket]
enabled = true

[middlewares.ops.ListBuckets]
enabled = true

[[middlewares]]
type = "identity"

# Client configuration
[client]
# Client type. Available: [s3]
type = "s3"
# S3 Endpoint Url
endpointUrl = "http://localhost:9000"
# Set bucket via path or subdomain
# true: http://localhost:9000/<bucket>/
# false: http://<bucket>.localhost:9000/
forcePathStyle = true
# send requests over HTTP/2
enableHttp2 = true
# allow insecure requests
insecure = true
# connection timeout
connectTimeout = 10_000
# read timeout
readTimeout = 10_000
# Timeout per operation, including retries
operationTimeout = 10_000
# Timeout per single operation, not including retries
operationAttemptTimeout = 10_000
# Maximum retry attempts
maxRetryAttempts = 3

# Client credentails
[client.credentials]
accessKeyId = "user"
secretKey = "password"
```

# Start MinIO

```
cd project/s3p
podman-compose up -d
```

# Benchmarks

### Requirements

Install instructions are provided for Rocky Linux 9

xpath: `dnf -y install perl-XML-XPath`
iftop: `dnf -y install epel-release iftop`
warp: https://github.com/minio/warp
oha: https://github.com/hatoo/oha 

## Prepare

Create a bucket called `bench` on the S3 server and set the access to public
The prepare script will then create the dataset
`./prepare.sh <server>`

## Run test
Runs the tests with all sizes
`./run.sh -s <server> -p <port> -i <interface to capture> -u <port to capture> -n <name of measurement> [oha|warp]`

## Example

Runs minIO and Warp on a local MinIO server running in podman

`./run.sh -s 127.0.0.1 -p 9000 -i podman1 -u 9000 -n local-minio`

## Analysis

### Requirements

`pip install pandas ods pprint pyexcel`

### Run

This will gather data from all collected outputs and collect them in a .ods file
`python analyze.py <path/to/outputs>`