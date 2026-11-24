// Export a local game binary's recovered functions as C pseudocode.
//@category Workshop
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.FunctionIterator;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Locale;

public class WorkshopGameFunctions extends GhidraScript {
    @Override public void run() throws Exception {
        Path dir = getSourceFile().getFile(false).toPath().getParent();
        String filter = Files.readString(dir.resolve("filter.txt")).trim().toLowerCase(Locale.ROOT);
        DecompInterface decompiler = new DecompInterface();
        int written = 0, skipped = 0;
        boolean reachedLimit = false;
        Path output = dir.resolve("functions.c");
        try (var writer = Files.newBufferedWriter(output)) {
            if (!decompiler.openProgram(currentProgram)) {
                throw new IllegalStateException(decompiler.getLastMessage());
            }
            FunctionIterator functions = currentProgram.getFunctionManager().getFunctions(true);
            while (functions.hasNext() && written+skipped < 64 && !monitor.isCancelled()) {
                var function = functions.next();
                if (function.isExternal() || !function.getName().toLowerCase(Locale.ROOT).contains(filter)) continue;
                var result = decompiler.decompileFunction(function, 12, monitor);
                if (!result.decompileCompleted() || result.getDecompiledFunction() == null) { skipped++; continue; }
                writer.write("/* " + function.getEntryPoint() + " */\n");
                writer.write(result.getDecompiledFunction().getC());
                writer.write("\n\n");
                written++;
            }
            reachedLimit = functions.hasNext() && written + skipped == 64;
        } finally {
            decompiler.dispose();
        }
        if (written == 0) Files.deleteIfExists(output);
        Files.writeString(dir.resolve("summary.txt"), written == 0
            ? "No matching functions could be decompiled. Try a different name or leave the filter empty."
            : "C pseudocode · " + written + " function" + (written == 1 ? "" : "s")
                + (skipped == 0 ? "" : " · " + skipped + " unavailable")
                + (reachedLimit ? " · 64-function limit reached" : ""));
    }
}
