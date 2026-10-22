FROM docker.io/library/nginx:1.30.6-alpine
COPY deploy/telemetry/nginx/nginx.conf /etc/nginx/nginx.conf
COPY deploy/telemetry/nginx/*.template /etc/nginx/templates/
COPY deploy/telemetry/nginx/start.sh /usr/local/bin/start-telemetry-nginx
RUN chmod 0755 /usr/local/bin/start-telemetry-nginx
EXPOSE 8080 8443
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
	CMD ["wget", "-q", "-O", "/dev/null", "http://127.0.0.1:8081/health"]
ENTRYPOINT ["/usr/local/bin/start-telemetry-nginx"]
CMD ["nginx", "-g", "daemon off;"]
