# A yehi image for Linux machines and cloud jobs. Without a GPU passed through it renders on the CPU
# through Mesa's software Vulkan driver (lavapipe); with an NVIDIA GPU (--gpus all) it uses the GPU and NVENC.
FROM rust:1-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock about.toml about.hbs ./
COPY src src
COPY docs docs
COPY scripts scripts
RUN cargo build --release --locked && scripts/third-party-licenses.sh THIRD-PARTY-LICENSES.txt

FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ffmpeg mesa-vulkan-drivers libvulkan1 ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/yehi /usr/local/bin/yehi
COPY LICENSE /usr/share/doc/yehi/LICENSE
COPY --from=build /src/THIRD-PARTY-LICENSES.txt /usr/share/doc/yehi/THIRD-PARTY-LICENSES.txt
WORKDIR /work
ENTRYPOINT ["yehi"]
