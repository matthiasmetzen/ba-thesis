<div align="center">
  <img src="https://coconucos.cs.hhu.de/lehre/bigdata/resources/img/hhu-logo.svg" width=300>

  [![Download](https://img.shields.io/static/v1?label=&message=pdf&color=EE3F24&style=for-the-badge&logo=adobe-acrobat-reader&logoColor=FFFFFF)](/../-/jobs/artifacts/master/file/document/thesis.pdf?job=latex)
</div>

# :notebook: &nbsp; Aufgabenbeschreibung

_Hier den erstellten Entwurf einfügen_

# Building

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

## Prepare

Create a bucket called `bench` on the S3 server and set access to public

`./prepare.sh <server>`

## Warp

`./run.sh <server>`

## oha

Single object, 10KiB, 200KiB

`./oha "http://<host>:<port>/<bucket>/<key>" -z 10sec`