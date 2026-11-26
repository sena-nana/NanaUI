package app.nanaui.host;

import android.os.Bundle;
import android.view.Window;
import android.view.WindowManager;
import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsCompat;
import androidx.core.view.WindowInsetsControllerCompat;
import com.google.androidgamesdk.GameActivity;

public class NanaActivity extends GameActivity {
    static {
        System.loadLibrary("nana_android_host");
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        // Edge-to-edge by default: NanaUI pads the root by the reported safe area.
        WindowCompat.setDecorFitsSystemWindows(getWindow(), false);
    }

    public void nanaSetKeepScreenAwake(final boolean keepAwake) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                Window window = getWindow();
                if (window == null) {
                    return;
                }
                if (keepAwake) {
                    window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                } else {
                    window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                }
            }
        });
    }

    public void nanaSetSystemBarsHidden(final boolean hidden) {
        runOnUiThread(new Runnable() {
            @Override
            public void run() {
                Window window = getWindow();
                if (window == null) {
                    return;
                }
                WindowInsetsControllerCompat controller =
                    WindowCompat.getInsetsController(window, window.getDecorView());
                if (controller == null) {
                    return;
                }
                if (hidden) {
                    controller.setSystemBarsBehavior(
                        WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
                    controller.hide(WindowInsetsCompat.Type.systemBars());
                } else {
                    controller.show(WindowInsetsCompat.Type.systemBars());
                }
            }
        });
    }
}
