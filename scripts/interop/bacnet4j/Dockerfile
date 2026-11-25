FROM docker.io/library/maven@sha256:1a352420f7aba21f5ad08df31bab55f74c013fb491f1ae8ab1dd7ff9ed698584
WORKDIR /opt/peer
COPY pom.xml .
COPY src src
RUN mvn --batch-mode --no-transfer-progress compile dependency:copy-dependencies \
 && echo '588dc6e986dd118fee24d436420cbbb745aa1a7d9e8583e15e2d93deeb49e97f  target/dependency/bacnet4j-6.1.1.jar' | sha256sum -c - \
 && sha256sum target/dependency/*.jar target/classes/*.class > classpath.sha256
ENTRYPOINT ["java","-cp","target/classes:target/dependency/*","SegmentedPeer"]
