FROM debian:bookworm

ARG USERNAME=vscode
ARG USER_UID=1000
ARG USER_GID=$USER_UID

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
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

RUN svn checkout https://svn.code.sf.net/p/findent/code/trunk/findent /opt/findent \
    && cd /opt/findent \
    && ./configure --prefix=/usr/local \
    && make -j"$(nproc)" \
    && make check \
    && make install \
    && findent -h > /dev/null