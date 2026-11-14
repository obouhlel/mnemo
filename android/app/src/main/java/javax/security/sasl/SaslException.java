package javax.security.sasl;

import java.io.IOException;

/**
 * Mnemo : javax.security.sasl n'existe pas sur Android. Le pilote PostgreSQL (authentification SCRAM)
 * n'en utilise que cette exception, reprise ici avec la même forme que dans le JDK.
 */
public class SaslException extends IOException {
    private Throwable exception;

    public SaslException() {
        super();
    }

    public SaslException(String detail) {
        super(detail);
    }

    public SaslException(String detail, Throwable ex) {
        super(detail);
        if (ex != null) {
            initCause(ex);
        }
    }

    @Override
    public synchronized Throwable getCause() {
        return exception;
    }

    @Override
    public synchronized Throwable initCause(Throwable cause) {
        super.initCause(cause);
        exception = cause;
        return this;
    }

    @Override
    public String toString() {
        String answer = super.toString();
        if (exception != null && exception != this) {
            answer += " [Caused by " + exception.toString() + "]";
        }
        return answer;
    }
}
