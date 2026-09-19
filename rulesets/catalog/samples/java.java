// Exercises every rule in rulesets/catalog/java.toml.
import java.lang.reflect.Field;

class Sample {
    void open(Field f) throws Exception {
        f.setAccessible(true); // reflection
    }

    void swallow() {
        try {
            open(null);
        } catch (Exception e) {
        } // empty-catch
    }
}
