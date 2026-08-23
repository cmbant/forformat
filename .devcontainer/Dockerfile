FROM debian:bookworm

ARG USERNAME=vscode
ARG USER_UID=1000
ARG USER_GID=$USER_UID

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        flex \
        bison \
        gfortran \
        git \
        ripgrep \
        subversion \
        procps \
        sudo \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd --gid ${USER_GID} ${USERNAME} \
    && useradd --uid ${USER_UID} --gid ${USER_GID} -m -s /bin/bash ${USERNAME} \
    && echo "${USERNAME} ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/${USERNAME} \
    && chmod 0440 /etc/sudoers.d/${USERNAME} \
    && git config --system --add safe.directory '*'

# The compatibility oracle. Trunk lags the released line, so take the tarball:
# see docs/compatibility.md for what the pinned version is contracted to do.
ARG FINDENT_VERSION=4.3.8~pre01
RUN curl -fsSL -o /tmp/findent.tar.gz \
        "https://ratrabbit.nl/downloads/findent/findent-${FINDENT_VERSION}.tar.gz" \
    && mkdir -p /opt/findent \
    && tar xf /tmp/findent.tar.gz -C /opt/findent --strip-components=1 \
    && rm /tmp/findent.tar.gz \
    && cd /opt/findent \
    && ./configure --prefix=/usr/local \
    && make -j"$(nproc)" \
    && make check \
    && make install \
    && findent -h > /dev/null