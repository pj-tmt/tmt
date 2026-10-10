# Local runner for the Colab Rules emulator suite (suite.mjs). CI does not use Docker.
# Pins mirror extensions/tmt-office/typescript/services/office/Dockerfile: temurin 21 JRE,
# node 22, firebase-tools 15.29.0. Emulators only; never mount host Firebase credentials.
FROM eclipse-temurin:21-jre-jammy@sha256:54b382c9d790d3201004d4e3921f4eff7dc7d67e1ce21a2da8d8faaaf63c85fc AS java
FROM node:22.23.2-bookworm-slim@sha256:83f487e0a63425e5b4d146fb5e5be574bcbe1b7b843d3ebafdd95eaf7767a7e5
COPY --from=java /opt/java/openjdk /opt/java/openjdk
ENV JAVA_HOME=/opt/java/openjdk
ENV PATH="/opt/java/openjdk/bin:${PATH}"
RUN npm install --global firebase-tools@15.29.0
USER node
WORKDIR /home/node
RUN firebase setup:emulators:firestore --non-interactive
ENV GCLOUD_PROJECT=demo-tmt-colab
