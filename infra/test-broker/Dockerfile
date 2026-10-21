# Throwaway Phase 0 test broker: Mosquitto over WebSocket, behind Railway's HTTPS edge.
FROM eclipse-mosquitto:2.0.22

COPY mosquitto.conf /mosquitto/config/mosquitto.conf
COPY entrypoint.sh /entrypoint.sh
# Where the broker keeps retained messages (the app's shared schedules) across
# restarts. A Railway volume is mounted here; the entrypoint fixes its owner as well,
# since a fresh volume belongs to root.
RUN chmod +x /entrypoint.sh \
	&& mkdir -p /mosquitto/data \
	&& chown mosquitto:mosquitto /mosquitto/data

EXPOSE 8080
ENTRYPOINT ["/entrypoint.sh"]
