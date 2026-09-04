import java.util.Collections;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

// [Item 7] finalizers — violation only, nothing to pair against
class ResourceHolder {
    protected void finalize() throws Throwable {
        super.finalize();
    }
}

class ResourceHolderGood implements AutoCloseable {
    public void close() {
    }
}

// [Item 9/10] equals/hashCode/toString
class BadPoint {
    private final int x;
    private final int y;

    BadPoint(int x, int y) {
        this.x = x;
        this.y = y;
    }

    public boolean equals(Object o) {
        return o instanceof BadPoint p && p.x == x && p.y == y;
    }
}

class GoodPoint {
    private final int x;
    private final int y;

    GoodPoint(int x, int y) {
        this.x = x;
        this.y = y;
    }

    public boolean equals(Object o) {
        return o instanceof GoodPoint p && p.x == x && p.y == y;
    }

    public int hashCode() {
        return 31 * x + y;
    }

    public String toString() {
        return "GoodPoint(" + x + ", " + y + ")";
    }
}

// [Item 13/14] public fields, [Item 15] mutability, [Item 45/15] initialization
class BadState {
    public int badField;
    private int counter;
    private String name;

    BadState() {
    }
}

class GoodState {
    public static final int MAX_SIZE = 100;
    private final int limit = 10;
    private final String name;

    GoodState(String name) {
        this.name = name;
    }
}

// [Item 30] enum over int constants
class SeasonConstants {
    static final int WINTER = 0;
    static final int SPRING = 1;
    static final int SUMMER = 2;
    static final int FALL = 3;
}

enum Season {
    WINTER, SPRING, SUMMER, FALL
}

// [Item 43] null vs. empty collection
class Roster {
    List<String> getNames() {
        return null;
    }

    List<String> getNamesSafe() {
        return Collections.emptyList();
    }
}

// [Item 46] for-each over indexed for
class Printer {
    void printBad(List<String> names) {
        for (int i = 0; i < names.size(); i++) {
            System.out.println(names.get(i));
        }
    }

    void printGood(List<String> names) {
        for (String name : names) {
            System.out.println(name);
        }
    }
}

// [Item 49] primitives over boxed
class Counter {
    Integer badCount = 0;
    int goodCount = 0;
}

// [Item 68] executors over raw Thread
class Worker {
    void startBad() {
        new Thread(() -> System.out.println("work")).start();
    }

    void startGood() {
        ExecutorService pool = Executors.newFixedThreadPool(4);
        pool.submit(() -> System.out.println("work"));
    }
}

// [Item 74] Serializable judiciously
class ConfigBad implements Serializable {
    int value;
}

class ConfigGood implements Comparable {
    int value;

    public int compareTo(Object o) {
        return 0;
    }
}
