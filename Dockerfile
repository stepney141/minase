FROM nvidia/cuda:13.0.3-base-ubuntu24.04@sha256:7c7413a56200486f71f181cad9310f6fd31b6bb21816ade15fc9c1e1e927a5c1

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

COPY tools/train/zero/ /opt/minase-zero/

SHELL ["/bin/bash", "-euo", "pipefail", "-c"]

# Keep installation, cleanup, and permissions in one layer to avoid duplicating the environment.
RUN bash /opt/minase-zero/setup.sh \
    && source /opt/minase-zero/env.sh \
    && uv cache clean \
    && chmod -R a+rwX /opt/minase-zero

# env.sh defines installation paths and environment variables for each session.
# vast.ai SSH sessions may not inherit the image's environment.
RUN printf '%s\n' 'source /opt/minase-zero/env.sh' >> /etc/bash.bashrc
