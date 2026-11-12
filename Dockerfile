FROM nvidia/cuda:13.0.3-base-ubuntu24.04@sha256:7c7413a56200486f71f181cad9310f6fd31b6bb21816ade15fc9c1e1e927a5c1 AS base

RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        rsync \
        python3 \
        build-essential \
        pkg-config \
        git \
        xz-utils \
        procps \
        time \
    && rm -rf /var/lib/apt/lists/*

FROM base AS build

COPY tools/train/zero/ /opt/minase-zero/

SHELL ["/bin/bash", "-euo", "pipefail", "-c"]

RUN bash /opt/minase-zero/setup.sh \
    && source /opt/minase-zero/env.sh \
    && uv cache clean \
    && chmod -R a+rwX /opt/minase-zero

# GHCR downloads were limited to about 15 MB/s per connection.
# Split the environment into eight layers so Docker can download them in parallel
# and keep completed layers when an interrupted download is retried.
RUN find /opt/minase-zero -type f -print0 \
    | while IFS= read -r -d '' path; do \
        if [[ "$path" == *$'\t'* || "$path" == *$'\n'* ]]; then \
            printf 'Cannot split path containing a tab or newline: %q\n' "$path" >&2; \
            exit 1; \
        fi; \
    done; \
    total=$(find /opt/minase-zero -type f -printf '%s\n' \
        | awk '{ total += $1 } END { printf "%.0f\n", total }'); \
    part_size=$((total / 8 + 1)); \
    mkdir -p /parts/{1..7}; \
    size_sum=0; \
    find /opt/minase-zero -type f -printf '%P\t%s\n' \
        | LC_ALL=C sort -t $'\t' -k1,1 \
        | while IFS=$'\t' read -r path size; do \
            size_sum=$((size_sum + size)); \
            part=$((size_sum / part_size)); \
            if ((part > 0)); then \
                destination="/parts/$part/$path"; \
                mkdir -p "$(dirname "$destination")"; \
                mv "/opt/minase-zero/$path" "$destination"; \
            fi; \
        done; \
    chmod -R a+rwX /parts

FROM base

COPY --from=build /opt/minase-zero/ /opt/minase-zero/
COPY --from=build /parts/1/ /opt/minase-zero/
COPY --from=build /parts/2/ /opt/minase-zero/
COPY --from=build /parts/3/ /opt/minase-zero/
COPY --from=build /parts/4/ /opt/minase-zero/
COPY --from=build /parts/5/ /opt/minase-zero/
COPY --from=build /parts/6/ /opt/minase-zero/
COPY --from=build /parts/7/ /opt/minase-zero/

SHELL ["/bin/bash", "-euo", "pipefail", "-c"]

# env.sh defines installation paths and environment variables for each session.
# vast.ai SSH sessions may not inherit the image's environment.
# COPY creates /opt/minase-zero with mode 755; host users need write access.
# Change only this directory to avoid duplicating its contents in this layer.
RUN chmod a+rwX /opt/minase-zero \
    && printf '%s\n' 'source /opt/minase-zero/env.sh' >> /etc/bash.bashrc
