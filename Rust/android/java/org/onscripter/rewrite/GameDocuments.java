package org.onscripter.rewrite;

import android.content.Context;
import android.content.SharedPreferences;
import android.database.Cursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.system.ErrnoException;
import android.system.Os;
import android.system.OsConstants;
import java.io.File;
import java.io.InputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.Locale;
import java.util.Map;

/** Lazy directory indexes and seekable handles; game data is never imported wholesale. */
final class GameDocuments {
    private static final int MAX_ENTRIES = 100000;
    private static final long COPY_LIMIT = 256L * 1024 * 1024;
    private final Context context;
    private final ArrayList<Uri> roots = new ArrayList<>();
    private final LinkedHashMap<Uri, Map<String,Uri>> directories = new LinkedHashMap<>(16,0.75f,true);
    private int entries;

    GameDocuments(Context context, SharedPreferences preferences) {
        this.context = context;
        for (String key : new String[] {"overlay","game"}) {
            String value = preferences.getString(key, "");
            if (!value.isEmpty()) roots.add(Uri.parse(value));
        }
    }
    int open(String name) throws IOException {
        String normalized = name.replace('\\','/');
        if (normalized.isEmpty() || normalized.startsWith("/") || normalized.indexOf('\0') >= 0 || normalized.indexOf(':') >= 0 || normalized.length() > 4095) throw new IOException("Asset name must be relative to the chosen folder.");
        ArrayList<String> components = new ArrayList<>();
        for (String part : normalized.split("/",-1)) {
            if (part.equals("..")) throw new IOException("Invalid asset path.");
            if (!part.isEmpty() && !part.equals(".")) components.add(part);
        }
        if (components.isEmpty()) throw new IOException("Asset path names a folder.");
        for (Uri tree : roots) {
            Uri current = DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree));
            for (String component : components) {
                current = children(tree,current).get(component.toLowerCase(Locale.ROOT));
                if (current == null) break;
            }
            if (current != null) return seekable(current);
        }
        return -1;
    }
    private Map<String,Uri> children(Uri tree, Uri parent) throws IOException {
        Map<String,Uri> cached = directories.get(parent);
        if (cached != null) return cached;
        HashMap<String,Uri> children = new HashMap<>();
        Uri query = DocumentsContract.buildChildDocumentsUriUsingTree(tree, DocumentsContract.getDocumentId(parent));
        String[] columns = {DocumentsContract.Document.COLUMN_DOCUMENT_ID, DocumentsContract.Document.COLUMN_DISPLAY_NAME};
        try (Cursor cursor = context.getContentResolver().query(query,columns,null,null,null)) {
            if (cursor == null) throw new IOException("Folder provider did not return a directory.");
            while (cursor.moveToNext()) {
                if (children.size() >= MAX_ENTRIES) throw new IOException("Directory index exceeds its memory budget.");
                String key = cursor.getString(1).toLowerCase(Locale.ROOT);
                Uri child = DocumentsContract.buildDocumentUriUsingTree(tree,cursor.getString(0));
                if (children.putIfAbsent(key,child) != null) throw new IOException("Folder contains ambiguous names: " + key);
            }
        }
        while (!directories.isEmpty() && (directories.size() >= 64 || entries + children.size() > MAX_ENTRIES)) {
            Uri eldest = directories.keySet().iterator().next(); entries -= directories.remove(eldest).size();
        }
        entries += children.size(); directories.put(parent,children); return children;
    }
    private int seekable(Uri uri) throws IOException {
        try (ParcelFileDescriptor descriptor = context.getContentResolver().openFileDescriptor(uri,"r")) {
            if (descriptor == null) throw new IOException("File provider returned no handle.");
            try { Os.lseek(descriptor.getFileDescriptor(),0,OsConstants.SEEK_CUR); return descriptor.detachFd(); }
            catch (ErrnoException error) { if (error.errno != OsConstants.ESPIPE) throw new IOException(error); }
            File temporary = File.createTempFile("ons-asset-", ".tmp",context.getCacheDir());
            try {
                try (InputStream input = new ParcelFileDescriptor.AutoCloseInputStream(descriptor); FileOutputStream output = new FileOutputStream(temporary)) {
                    byte[] buffer = new byte[65536]; long total = 0; int count;
                    while ((count = input.read(buffer)) != -1) {
                        total += count; if (total > COPY_LIMIT) throw new IOException("Non-seekable asset exceeds the 256 MiB copy limit; use locally stored game data.");
                        output.write(buffer,0,count);
                    }
                }
                try (ParcelFileDescriptor copied = ParcelFileDescriptor.open(temporary,ParcelFileDescriptor.MODE_READ_ONLY)) { return copied.detachFd(); }
            } finally { temporary.delete(); }
        }
    }
}
