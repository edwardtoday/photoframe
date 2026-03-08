FROM espressif/idf:v5.5.1

USER root
ENV CARGO_HOME=/root/.cargo \
    RUSTUP_HOME=/root/.rustup \
    PATH=/root/.cargo/bin:${PATH}

RUN curl https://sh.rustup.rs -sSf | bash -s -- -y --default-toolchain stable --profile minimal
RUN cargo install --locked ldproxy
RUN set -eux; \
    arch="$(uname -m)"; \
    case "$arch" in \
      aarch64) espup_arch='aarch64-unknown-linux-gnu'; espflash_arch='aarch64-unknown-linux-gnu' ;; \
      x86_64) espup_arch='x86_64-unknown-linux-gnu'; espflash_arch='x86_64-unknown-linux-gnu' ;; \
      *) echo "unsupported arch: $arch" >&2; exit 1 ;; \
    esac; \
    curl -L "https://github.com/esp-rs/espup/releases/latest/download/espup-${espup_arch}" -o /usr/local/bin/espup; \
    chmod +x /usr/local/bin/espup; \
    curl -L "https://github.com/esp-rs/espflash/releases/latest/download/espflash-${espflash_arch}.zip" -o /tmp/espflash.zip; \
    python3 -c "from zipfile import ZipFile; ZipFile('/tmp/espflash.zip').extract('espflash', '/usr/local/bin')"; \
    chmod +x /usr/local/bin/espflash; \
    rm -f /tmp/espflash.zip
RUN espup install --std --targets esp32s3 --export-file /root/export-esp.sh

ENTRYPOINT ["/bin/bash", "-lc"]
