# ArduPlane SITL for the T1-S sim (JSON physics backend).
#   docker build -t t1s-ardupilot-sitl ardupilot/
#   see ardupilot/run_sitl.sh
FROM ubuntu:24.04

ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        git ca-certificates build-essential g++ ccache gawk python3 python3-pip python3-dev \
        python3-venv libxml2-dev libxslt1-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN python3 -m venv /venv
ENV PATH=/venv/bin:$PATH
RUN pip install --no-cache-dir empy==3.3.4 pexpect future lxml pymavlink MAVProxy dronecan setuptools

ARG AP_REF=master
RUN git clone --depth 1 --branch ${AP_REF} --recurse-submodules --shallow-submodules \
        https://github.com/ArduPilot/ardupilot.git /ardupilot

WORKDIR /ardupilot
RUN ./waf configure --board sitl && ./waf plane

COPY t1s.parm t1s-vtol.parm /
COPY start.sh /start.sh
COPY check_link.py /check_link.py
RUN chmod +x /start.sh

# MAVLink for ground stations (QGroundControl / Mission Planner / MAVProxy).
EXPOSE 5760/tcp
ENTRYPOINT ["/start.sh"]
