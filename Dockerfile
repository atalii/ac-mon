 FROM rust:1.67

WORKDIR /usr/src/ac_mon
COPY . .
COPY ./test-conf.kdl /test-conf.kdl

RUN cargo install --path .

RUN apt-get update && apt-get install -y iproute2 net-tools

ENV RUST_LOG info

EXPOSE 8080

CMD ["ac-mon"] 
