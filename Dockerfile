FROM ubuntu:latest
COPY target/release/webhook-helper /bin/webhook-helper
RUN chmod a+x /bin/webhook-helper
ENTRYPOINT [ "webhook-helper" ]