#ifndef MEMREADER_SHIMS_H
#define MEMREADER_SHIMS_H

#include <linux/types.h>
#include <linux/wait.h>

#define THUB_MAX_TARGETS 64
#define THUB_MAX_RANGES_PER_JOB 32
#define THUB_DEFAULT_RING_BYTES (16 * 1024 * 1024)
#define THUB_MAX_PAYLOAD_BYTES (1024 * 1024)
#define THUB_TASK_COMM_BYTES 16
#define THUB_MODULE_PATH_BYTES 256

#define THUB_RECORD_KIND_DATA 1
#define THUB_RECORD_KIND_PADDING 2

#define THUB_STATUS_OK 0
#define THUB_STATUS_NO_TASK 1
#define THUB_STATUS_START_TIME_MISMATCH 2
#define THUB_STATUS_NO_MM 3
#define THUB_STATUS_PARTIAL_READ 4
#define THUB_STATUS_RING_FULL 5
#define THUB_STATUS_BAD_TARGET_SLOT 6
#define THUB_STATUS_BAD_REQUEST 7
#define THUB_STATUS_INTERNAL 8

#define THUB_RANGE_STATUS_OK 0
#define THUB_RANGE_STATUS_PARTIAL 1
#define THUB_RANGE_STATUS_FAULT 2

struct thub_target_selector {
	__u32 kind;
	__u32 flags;
	__u32 host_pid;
	__u32 pid_in_ns;
	__u32 container_init_host_pid;
	__u32 reserved0;
	__u64 start_time_ticks;
};

struct thub_upsert_target_request {
	__u32 slot;
	__u32 flags;
	struct thub_target_selector selector;
};

struct thub_remove_target_request {
	__u32 slot;
	__u32 reserved0;
};

struct thub_advance_tail_request {
	__u64 consumer_tail;
};

struct thub_read_range {
	__u64 remote_addr;
	__u32 len;
	__u32 reserved0;
};

struct thub_submit_read_request {
	__u32 target_slot;
	__u32 flags;
	__u64 cookie;
	__u32 range_count;
	__u32 reserved0;
	struct thub_read_range ranges[THUB_MAX_RANGES_PER_JOB];
};

struct thub_get_info_response {
	__u32 uapi_version;
	__u32 ring_mapping_bytes;
	__u32 max_targets;
	__u32 max_ranges_per_job;
	__u32 max_payload_bytes;
	__u32 reserved0;
};

struct thub_resolve_host_process_request {
	__u8 process_name[THUB_TASK_COMM_BYTES];
	__u32 match_count;
	__u32 pid;
	__u32 reserved0;
	__u64 start_time_ticks;
};

struct thub_inspect_host_process_request {
	__u32 pid;
	__u32 reserved0;
	__u8 process_name[THUB_TASK_COMM_BYTES];
	__u64 start_time_ticks;
};

struct thub_module_entry {
	__u64 base;
	__u64 end;
	__u64 file_offset;
	__u8 perms[4];
	__u32 path_len;
	__u16 reserved0;
	__u16 reserved1;
	__u8 path[THUB_MODULE_PATH_BYTES];
};

struct thub_list_modules_request {
	struct thub_target_selector selector;
	__u64 entries_ptr;
	__u32 capacity;
	__u32 returned;
	__u32 total_matches;
	__u32 reserved0;
};

struct thub_ring_layout {
	__u32 uapi_version;
	__u32 header_bytes;
	__u32 capacity_bytes;
	__u32 reserved0;
	__u64 producer_head;
	__u64 consumer_tail;
	__u64 dropped_records;
	__u64 next_seq;
	__u64 reserved1[4];
};

struct thub_range_result {
	__u64 remote_addr;
	__u32 requested_len;
	__u32 bytes_read;
	__u32 status;
	__u32 reserved0;
};

struct thub_record_header {
	__u32 total_len;
	__u32 kind;
	__u64 seq;
	__u64 timestamp_ns;
	__u64 cookie;
	__u32 target_slot;
	__u32 status;
	__u32 range_count;
	__u32 reserved0;
	__u32 payload_bytes;
	__u32 reserved1;
};

struct thub_target_slot {
	__u8 in_use;
	__u8 reserved[7];
	struct thub_target_selector selector;
};

struct thub_session {
	struct thub_ring_layout *layout;
	void *mapping;
	size_t mapping_bytes;
	wait_queue_head_t waitq;
	u8 *scratch;
	size_t scratch_len;
	struct thub_target_slot slots[THUB_MAX_TARGETS];
	size_t ring_capacity_bytes;
	__u64 producer_head;
	__u64 consumer_tail;
	__u64 dropped_records;
	__u64 next_seq;
};

int thub_rust_session_init(struct thub_session *session, size_t mapping_bytes);
int thub_rust_upsert_target(struct thub_session *session, const struct thub_upsert_target_request *req);
int thub_rust_remove_target(struct thub_session *session, const struct thub_remove_target_request *req);
int thub_rust_publish_record(struct thub_session *session, const struct thub_submit_read_request *req,
			     __u32 status, __u32 range_count,
			     const struct thub_range_result *range_results,
			     const u8 *payload, size_t payload_bytes);

int thub_kernel_read_target(struct thub_session *session,
			    const struct thub_submit_read_request *req,
			    struct thub_range_result *range_results,
			    size_t max_results,
			    size_t *payload_bytes,
			    __u32 *status_out);
int thub_kernel_advance_tail(struct thub_session *session,
			     const struct thub_advance_tail_request *req);
int thub_kernel_resolve_host_process(struct thub_resolve_host_process_request *req);
int thub_kernel_inspect_host_process(struct thub_inspect_host_process_request *req);
int thub_kernel_list_modules(struct thub_list_modules_request *req);
void thub_kernel_wake_consumer(struct thub_session *session);

#endif
