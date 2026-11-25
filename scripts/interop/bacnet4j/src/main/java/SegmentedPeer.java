import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.io.PrintWriter;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;

import com.serotonin.bacnet4j.LocalDevice;
import com.serotonin.bacnet4j.RemoteDevice;
import com.serotonin.bacnet4j.event.DeviceEventAdapter;
import com.serotonin.bacnet4j.exception.ErrorAPDUException;
import com.serotonin.bacnet4j.npdu.ip.IpNetworkBuilder;
import com.serotonin.bacnet4j.npdu.ip.IpNetworkUtils;
import com.serotonin.bacnet4j.service.confirmed.ConfirmedCovNotificationRequest;
import com.serotonin.bacnet4j.service.unconfirmed.WhoIsRequest;
import com.serotonin.bacnet4j.transport.DefaultTransport;
import com.serotonin.bacnet4j.type.constructed.PropertyValue;
import com.serotonin.bacnet4j.type.constructed.SequenceOf;
import com.serotonin.bacnet4j.type.enumerated.ObjectType;
import com.serotonin.bacnet4j.type.enumerated.PropertyIdentifier;
import com.serotonin.bacnet4j.type.primitive.CharacterString;
import com.serotonin.bacnet4j.type.primitive.ObjectIdentifier;
import com.serotonin.bacnet4j.type.primitive.UnsignedInteger;

/** Test-only loopback device. BACnet4J handles every BACnet request and response. */
public final class SegmentedPeer {
    private static final String HOST = "127.0.0.1";
    private static final int INSTANCE = 9001;
    private static final int ENDPOINT = 100;
    private final LocalDevice device;
    private final CompletableFuture<RemoteDevice> announced = new CompletableFuture<>();

    private SegmentedPeer() {
        var network = new IpNetworkBuilder().withLocalBindAddress(HOST)
                .withBroadcast("127.255.255.255", 8).withPort(47809).build();
        var transport = new DefaultTransport(network);
        transport.setSegWindow(2);
        device = new LocalDevice(INSTANCE, transport);
        device.writePropertyInternal(PropertyIdentifier.description, new CharacterString("D".repeat(6000)));
        device.getEventHandler().addListener(new DeviceEventAdapter() {
            @Override public void iAmReceived(RemoteDevice remote) {
                if (remote.getInstanceNumber() == ENDPOINT) announced.complete(remote);
            }
        });
    }

    private String command(String line) throws Exception {
        if ("INFO".equals(line)) {
            return "JDK " + Runtime.version() + "|" + Files.readString(Path.of("classpath.sha256")).strip().replace("\n", "|");
        }
        if (line.startsWith("DISCOVER ")) {
            int relayPort = Integer.parseInt(line.substring(9));
            var address = IpNetworkUtils.toAddress(HOST, relayPort);
            device.send(address, new WhoIsRequest(ENDPOINT, ENDPOINT));
            var remote = announced.get(20, TimeUnit.SECONDS);
            if (!remote.getAddress().equals(address)) throw new IllegalStateException("I-Am address differs from relay");
            return "IAM " + remote.getInstanceNumber() + " " + remote.getMaxAPDULengthAccepted()
                    + " " + remote.getSegmentationSupported().intValue();
        }
        if ("COV 7".equals(line) || "COV 9".equals(line)) {
            int process = Integer.parseInt(line.substring(4));
            var remote = announced.get(1, TimeUnit.SECONDS);
            var oid = new ObjectIdentifier(ObjectType.device, INSTANCE);
            var description = new CharacterString((process == 7 ? "A" : "U").repeat(6000));
            var notification = new ConfirmedCovNotificationRequest(new UnsignedInteger(process), oid, oid,
                    new UnsignedInteger(60), new SequenceOf<>(new PropertyValue(PropertyIdentifier.description, description)));
            try {
                device.send(remote, notification).get();
                return "SIMPLE_ACK";
            } catch (ErrorAPDUException error) {
                var cause = error.getError();
                return "ERROR " + cause.getErrorClass().intValue() + " " + cause.getErrorCode().intValue();
            }
        }
        throw new IllegalArgumentException("unrecognized command");
    }

    private void run() throws Exception {
        device.initialize();
        try (var server = new ServerSocket(47811, 4, InetAddress.getByName(HOST))) {
            System.out.println("BACnet4J peer ready");
            while (true) {
                try (Socket socket = server.accept();
                     var reader = new BufferedReader(new InputStreamReader(socket.getInputStream(), StandardCharsets.US_ASCII));
                     var writer = new PrintWriter(socket.getOutputStream(), true, StandardCharsets.US_ASCII)) {
                    socket.setSoTimeout(30000);
                    try {
                        writer.println(command(reader.readLine()));
                    } catch (Exception error) {
                        error.printStackTrace(System.err);
                        writer.println("FAILED " + error);
                    }
                }
            }
        } finally {
            device.terminate();
        }
    }

    public static void main(String[] args) throws Exception {
        new SegmentedPeer().run();
    }
}
