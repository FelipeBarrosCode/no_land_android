import android.os.SystemClock;
import android.view.InputDevice;
import android.view.InputEvent;
import android.view.MotionEvent;
import java.lang.reflect.Method;

/** Shell-only device smoke helper: injects a two/three-finger tap, never packaged in the app. */
public final class InjectStreamTouch {
  public static void main(String[] args) throws Exception {
    int count = Integer.parseInt(args[0]);
    if (count < 2 || count > 3) throw new IllegalArgumentException("Use 2 or 3 fingers");
    float x = Float.parseFloat(args[1]);
    float y = Float.parseFloat(args[2]);
    Class<?> type;
    try { type = Class.forName("android.hardware.input.InputManagerGlobal"); }
    catch (ClassNotFoundException oldAndroid) { type = Class.forName("android.hardware.input.InputManager"); }
    Object manager = type.getMethod("getInstance").invoke(null);
    Method inject = type.getMethod("injectInputEvent", InputEvent.class, int.class);
    MotionEvent.PointerProperties[] properties = new MotionEvent.PointerProperties[count];
    MotionEvent.PointerCoords[] coordinates = new MotionEvent.PointerCoords[count];
    for (int i = 0; i < count; i++) {
      properties[i] = new MotionEvent.PointerProperties();
      properties[i].id = i;
      properties[i].toolType = MotionEvent.TOOL_TYPE_FINGER;
      coordinates[i] = new MotionEvent.PointerCoords();
      coordinates[i].x = x + i * 100;
      coordinates[i].y = y;
      coordinates[i].pressure = 1;
      coordinates[i].size = 0.1f;
    }
    long down = SystemClock.uptimeMillis();
    for (int active = 1; active <= count; active++) {
      int action = active == 1 ? MotionEvent.ACTION_DOWN : MotionEvent.ACTION_POINTER_DOWN |
        ((active - 1) << MotionEvent.ACTION_POINTER_INDEX_SHIFT);
      send(manager, inject, down, action, active, properties, coordinates);
      SystemClock.sleep(25);
    }
    SystemClock.sleep(50);
    for (int active = count; active >= 1; active--) {
      int action = active == 1 ? MotionEvent.ACTION_UP : MotionEvent.ACTION_POINTER_UP |
        ((active - 1) << MotionEvent.ACTION_POINTER_INDEX_SHIFT);
      send(manager, inject, down, action, active, properties, coordinates);
      SystemClock.sleep(25);
    }
    System.out.println("Injected " + count + "-finger tap");
  }

  private static void send(Object manager, Method inject, long down, int action, int count,
      MotionEvent.PointerProperties[] properties, MotionEvent.PointerCoords[] coordinates) throws Exception {
    MotionEvent event = MotionEvent.obtain(down, SystemClock.uptimeMillis(), action, count,
      properties, coordinates, 0, 0, 1, 1, 0, 0, InputDevice.SOURCE_TOUCHSCREEN, 0);
    try {
      if (!Boolean.TRUE.equals(inject.invoke(manager, event, 2))) throw new IllegalStateException("Input injection rejected");
    } finally { event.recycle(); }
  }
}
