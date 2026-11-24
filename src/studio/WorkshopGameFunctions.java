// Export a local game binary's recovered functions as C pseudocode.
//@category Workshop
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.FunctionIterator;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.nio.charset.StandardCharsets;
import java.io.BufferedWriter;
import java.time.Instant;
import java.time.temporal.ChronoUnit;
import java.util.Locale;

public class WorkshopGameFunctions extends GhidraScript {
    private void event(BufferedWriter log, String message) throws Exception {
        log.write(Instant.now().truncatedTo(ChronoUnit.SECONDS) + "  " + message.replace('\n', ' ').replace('\r', ' '));
        log.newLine();
        log.flush();
    }
    @Override public void run() throws Exception {
        Path script = getSourceFile().getFile(false).toPath().getParent();
        Path dir = Path.of(getScriptArgs()[0]);
        String filter = Files.readString(script.resolve("filter.txt")).trim().toLowerCase(Locale.ROOT);
        DecompInterface decompiler = new DecompInterface();
        int written = 0, skipped = 0;
        boolean reachedLimit = false;
        Path output = dir.resolve("functions.c");
        long offset = 0;
        try (var writer = Files.newOutputStream(output);
             var symbols = Files.newBufferedWriter(dir.resolve("symbols.tsv"));
             var log = Files.newBufferedWriter(dir.resolve("activity.log"), StandardOpenOption.CREATE, StandardOpenOption.APPEND)) {
            event(log,"Imported " + currentProgram.getName() + " · " + currentProgram.getLanguageID()
                + " · " + currentProgram.getFunctionManager().getFunctionCount() + " known functions");
            if (!decompiler.openProgram(currentProgram)) {
                throw new IllegalStateException(decompiler.getLastMessage());
            }
            FunctionIterator functions = currentProgram.getFunctionManager().getFunctions(true);
            while (functions.hasNext() && !monitor.isCancelled()) {
                var function = functions.next();
                if (function.isExternal() || !function.getName().toLowerCase(Locale.ROOT).contains(filter)) continue;
                if (written + skipped == 64) { reachedLimit = true; break; }
                String name = function.getName().replace('\t',' ').replace('\n',' ').replace('\r',' ');
                String label = name + " @ " + function.getEntryPoint();
                event(log, "Reading " + label);
                long started = System.nanoTime();
                var result = decompiler.decompileFunction(function, 12, monitor);
                if (!result.decompileCompleted() || result.getDecompiledFunction() == null) {
                    skipped++;
                    event(log, "Skipped " + label + " · " + result.getErrorMessage());
                    continue;
                }
                byte[] entry = ("/* " + function.getEntryPoint() + " */\n" + result.getDecompiledFunction().getC() + "\n\n").getBytes(StandardCharsets.UTF_8);
                writer.write(entry);
                symbols.write(offset + "\t" + function.getEntryPoint() + "\t" + name + "\n");
                offset += entry.length;
                written++;
                event(log, "Saved " + label + " · " + entry.length + " bytes in " + (System.nanoTime()-started)/1_000_000 + " ms");
            }
            monitor.checkCancelled();
            event(log, written + " saved, " + skipped + " unavailable" + (reachedLimit ? " · 64-function limit reached" : ""));
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
