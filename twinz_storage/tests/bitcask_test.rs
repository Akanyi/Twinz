use std::sync::Arc;
use tokio::fs;
use tokio::fs::File; // Added for File::create
use twinz_storage::{BitCask, BitCaskOptions};

#[tokio::test]
async fn test_bitcask_basic_io() {
    let test_dir = "./test_data_basic";
    // Cleanup first
    if fs::metadata(test_dir).await.is_ok() {
        fs::remove_dir_all(test_dir).await.unwrap();
    }

    let storage = BitCask::open(test_dir, BitCaskOptions::default())
        .await
        .unwrap();

    // Test Put
    let key = b"name".to_vec();
    let value = b"twinz".to_vec();
    storage.put(key.clone(), value.clone()).await.unwrap();

    // Test Get
    let read_value = storage.get(&key).await.unwrap();
    assert_eq!(read_value, value);

    // Clean up
    fs::remove_dir_all(test_dir).await.unwrap();
}

#[tokio::test]
async fn test_bitcask_persistance() {
    let test_dir = "./test_data_persistance";
    if fs::metadata(test_dir).await.is_ok() {
        fs::remove_dir_all(test_dir).await.unwrap();
    }

    // SCOPE 1: Write data
    {
        let storage = BitCask::open(test_dir, BitCaskOptions::default())
            .await
            .unwrap();
        storage.put(b"k1".to_vec(), b"v1".to_vec()).await.unwrap();
        storage.put(b"k2".to_vec(), b"v2".to_vec()).await.unwrap();
    } // Drop storage, close files

    // SCOPE 2: Re-open and verify
    {
        let storage = BitCask::open(test_dir, BitCaskOptions::default())
            .await
            .unwrap();

        let v1 = storage.get(b"k1").await.unwrap();
        assert_eq!(v1, b"v1");

        let v2 = storage.get(b"k2").await.unwrap();
        assert_eq!(v2, b"v2");
    }

    fs::remove_dir_all(test_dir).await.unwrap();
}

#[tokio::test]
async fn test_bitcask_compaction() {
    let test_dir = "./test_data_compact";
    if fs::metadata(test_dir).await.is_ok() {
        fs::remove_dir_all(test_dir).await.unwrap();
    }

    // 1. Write Data (Active File 0)
    {
        let storage = BitCask::open(test_dir, BitCaskOptions::default())
            .await
            .unwrap();
        storage
            .put(b"k1".to_vec(), b"old_v1".to_vec())
            .await
            .unwrap(); // Invalid
        storage
            .put(b"k1".to_vec(), b"new_v1".to_vec())
            .await
            .unwrap(); // Valid
        storage.put(b"k2".to_vec(), b"v2".to_vec()).await.unwrap(); // Valid
    }

    // 2. Manual Rotation: Rename 0.data -> 10.data (Historical), Create 11.data (Active)
    {
        let old_file = format!("{}/0.data", test_dir);
        let history_file = format!("{}/10.data", test_dir);
        let new_active = format!("{}/11.data", test_dir);
        fs::rename(old_file, history_file).await.unwrap();
        File::create(new_active).await.unwrap();
    }

    // 3. Compact and Verify
    {
        let storage = BitCask::open(test_dir, BitCaskOptions::default())
            .await
            .unwrap();
        storage.compact().await.unwrap();

        // Verify Data
        let v1 = storage.get(b"k1").await.unwrap();
        assert_eq!(v1, b"new_v1");

        let v2 = storage.get(b"k2").await.unwrap();
        assert_eq!(v2, b"v2");

        // Verify File Deletion
        let path_10 = format!("{}/10.data", test_dir);
        assert!(
            fs::metadata(path_10).await.is_err(),
            "Historical file should be deleted"
        );
    }

    // Clean up
    if fs::metadata(test_dir).await.is_ok() {
        fs::remove_dir_all(test_dir).await.unwrap();
    }
}
