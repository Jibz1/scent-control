# Throwaway Phase 0 test broker: Mosquitto over WebSocket, behind Railway's HTTPS edge.
FROM eclipse-mosquitto:2.0.22

COPY mosquitto.conf /mosquitto/config/mosquitto.conf
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

EXPOSE 8080
ENTRYPOINT ["/entrypoint.sh"]
