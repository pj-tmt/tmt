# Local and CI runner for the Colab Rules emulator suite (suite.mjs).
# Emulator prerequisites: temurin 21 JRE,
# node 26.12.1, firebase-tools 15.29.0. Emulators only; never mount host Firebase credentials.
FROM eclipse-temurin:21-jre-jammy@sha256:54b382c9d790d3201004d4e3921f4eff7dc7d67e1ce21a2da8d8faaaf63c85fc AS java
FROM node:26.12.1-bookworm-slim@sha256:28147155d47dc31abe0a46e0d12b59b2fb3d0a2dcc5dc65fce76a074180b16fe
COPY --from=java /opt/java/openjdk /opt/java/openjdk
ENV JAVA_HOME=/opt/java/openjdk
ENV PATH="/opt/java/openjdk/bin:${PATH}"
RUN npm install --global firebase-tools@15.29.0
USER node
WORKDIR /home/node
RUN firebase setup:emulators:firestore --non-interactive
ENV GCLOUD_PROJECT=demo-tmt-colab
