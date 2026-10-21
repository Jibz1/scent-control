#!/bin/sh
set -eu
if [ -z "${TEST_BROKER_USERNAME:-}" ] || [ -z "${TEST_BROKER_PASSWORD:-}" ]; then
	echo "TEST_BROKER_USERNAME and TEST_BROKER_PASSWORD must be set" >&2
	exit 1
fi
mosquitto_passwd -c -b /mosquitto/config/passwd "$TEST_BROKER_USERNAME" "$TEST_BROKER_PASSWORD"
chown mosquitto:mosquitto /mosquitto/config/passwd
chmod 0700 /mosquitto/config/passwd
# The persistence directory (see mosquitto.conf). A volume mounted over it starts out
# owned by root, and Mosquitto drops to the mosquitto user before it writes there.
mkdir -p /mosquitto/data
chown mosquitto:mosquitto /mosquitto/data
exec mosquitto -c /mosquitto/config/mosquitto.conf
