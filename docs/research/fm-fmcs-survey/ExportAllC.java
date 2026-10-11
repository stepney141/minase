// Export decompiled C for every function to a single file.
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.address.*;
import java.io.*;

public class ExportAllC extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        String outPath = args.length > 0 ? args[0] : "decompiled.c";
        DecompInterface ifc = new DecompInterface();
        DecompileOptions opts = new DecompileOptions();
        ifc.setOptions(opts);
        ifc.openProgram(currentProgram);
        PrintWriter w = new PrintWriter(new FileWriter(outPath));
        FunctionIterator it = currentProgram.getFunctionManager().getFunctions(true);
        int n = 0;
        while (it.hasNext() && !monitor.isCancelled()) {
            Function f = it.next();
            if (f.isThunk()) continue;
            DecompileResults r = ifc.decompileFunction(f, 120, monitor);
            w.println("// ==== FUNCTION " + f.getName() + " @ " + f.getEntryPoint() + " size=" + f.getBody().getNumAddresses());
            if (r != null && r.decompileCompleted()) {
                w.println(r.getDecompiledFunction().getC());
            } else {
                w.println("// decompile failed: " + (r == null ? "null" : r.getErrorMessage()));
            }
            n++;
        }
        w.close();
        ifc.dispose();
        println("exported " + n + " functions to " + outPath);
    }
}
