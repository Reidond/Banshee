// Stage the Windows App SDK bootstrap DLL (and resources.pri) next to the
// built exe so `windows_reactor::bootstrap()` can find the machine-installed
// Microsoft.WindowsAppRuntime.2.x. Framework-dependent (not self-contained):
// no NuGet download at build time, consumes the runtime already on the box.
fn main() {
    windows_reactor_setup::as_framework_dependent();
}
