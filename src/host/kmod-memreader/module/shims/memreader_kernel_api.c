// SPDX-License-Identifier: GPL-2.0
#include <linux/jiffies.h>
#include <linux/dcache.h>
#include <linux/fs.h>
#include <linux/mm.h>
#include <linux/mm_types.h>
#include <linux/pid.h>
#include <linux/pid_namespace.h>
#include <linux/math64.h>
#include <linux/path.h>
#include <linux/sched/mm.h>
#include <linux/sched/signal.h>
#include <linux/sched/task.h>
#include <linux/timekeeping.h>
#include <linux/const.h>
#include <asm/param.h>

#include "../include/memreader_shims.h"

static u64 thub_task_start_time_ticks(const struct task_struct *task)
{
	return div_u64(task->start_boottime * USER_HZ, NSEC_PER_SEC);
}

static struct task_struct *thub_lookup_task(const struct thub_target_selector *selector)
{
	struct task_struct *task = NULL;

	switch (selector->kind) {
	case 1:
		task = get_pid_task(find_vpid(selector->host_pid), PIDTYPE_PID);
		break;
	case 2: {
		struct task_struct *init_task;
		struct pid_namespace *ns;

		init_task = get_pid_task(find_vpid(selector->container_init_host_pid), PIDTYPE_PID);
		if (!init_task)
			return NULL;
		ns = task_active_pid_ns(init_task);
		task = get_pid_task(find_pid_ns(selector->pid_in_ns, ns), PIDTYPE_PID);
		put_task_struct(init_task);
		break;
	}
	default:
		break;
	}

	if (!task)
		return NULL;

	if (thub_task_start_time_ticks(task) != selector->start_time_ticks) {
		put_task_struct(task);
		return ERR_PTR(-ESTALE);
	}

	return task;
}

static void thub_copy_task_comm(__u8 out[THUB_TASK_COMM_BYTES], const struct task_struct *task)
{
	memset(out, 0, THUB_TASK_COMM_BYTES);
	strscpy(out, task->comm, THUB_TASK_COMM_BYTES);
}

static void thub_fill_vma_perms(const struct vm_area_struct *vma, __u8 out[4])
{
	out[0] = (vma->vm_flags & VM_READ) ? 'r' : '-';
	out[1] = (vma->vm_flags & VM_WRITE) ? 'w' : '-';
	out[2] = (vma->vm_flags & VM_EXEC) ? 'x' : '-';
	out[3] = (vma->vm_flags & VM_SHARED) ? 's' : 'p';
}

int thub_kernel_resolve_host_process(struct thub_resolve_host_process_request *req)
{
	struct task_struct *task;
	char wanted[THUB_TASK_COMM_BYTES];

	if (!req)
		return -EINVAL;

	memcpy(wanted, req->process_name, THUB_TASK_COMM_BYTES);
	wanted[THUB_TASK_COMM_BYTES - 1] = '\0';
	if (!wanted[0])
		return -EINVAL;

	req->match_count = 0;
	req->pid = 0;
	req->start_time_ticks = 0;

	rcu_read_lock();
	for_each_process(task) {
		if (strncmp(task->comm, wanted, THUB_TASK_COMM_BYTES))
			continue;
		req->match_count++;
		if (req->match_count == 1) {
			req->pid = task_pid_nr(task);
			req->start_time_ticks = thub_task_start_time_ticks(task);
		}
	}
	rcu_read_unlock();

	if (req->match_count == 0)
		return -ESRCH;

	return 0;
}

int thub_kernel_inspect_host_process(struct thub_inspect_host_process_request *req)
{
	struct task_struct *task;

	if (!req || !req->pid)
		return -EINVAL;

	task = get_pid_task(find_vpid(req->pid), PIDTYPE_PID);
	if (!task)
		return -ESRCH;

	thub_copy_task_comm(req->process_name, task);
	req->start_time_ticks = thub_task_start_time_ticks(task);
	put_task_struct(task);
	return 0;
}

int thub_kernel_list_modules(struct thub_list_modules_request *req)
{
	struct task_struct *task;
	struct mm_struct *mm;
	struct vm_area_struct *vma;
	struct thub_module_entry __user *entries;
	char *path_buf = NULL;
	int rc = 0;
	__u32 returned = 0;
	__u32 total_matches = 0;

	if (!req)
		return -EINVAL;

	task = thub_lookup_task(&req->selector);
	if (IS_ERR(task))
		return PTR_ERR(task);
	if (!task)
		return -ESRCH;

	mm = get_task_mm(task);
	put_task_struct(task);
	if (!mm)
		return -ESRCH;

	entries = (__force struct thub_module_entry __user *)(uintptr_t)req->entries_ptr;
	path_buf = kmalloc(THUB_MODULE_PATH_BYTES, GFP_KERNEL);
	if (!path_buf) {
		mmput(mm);
		return -ENOMEM;
	}

	{
		VMA_ITERATOR(vmi, mm, 0);

		mmap_read_lock(mm);
		for_each_vma(vmi, vma) {
			struct thub_module_entry entry;
			char *resolved;
			size_t path_len;
			const char *path = "[anon]";

			if (vma->vm_file) {
				resolved = d_path(&vma->vm_file->f_path, path_buf, THUB_MODULE_PATH_BYTES);
				if (IS_ERR(resolved))
					continue;
				path = resolved;
			}

			total_matches++;
			if (returned >= req->capacity)
				continue;

			memset(&entry, 0, sizeof(entry));
			entry.base = vma->vm_start;
			entry.end = vma->vm_end;
			entry.file_offset = ((u64)vma->vm_pgoff) << PAGE_SHIFT;
			thub_fill_vma_perms(vma, entry.perms);
			path_len = strnlen(path, THUB_MODULE_PATH_BYTES);
			if (path_len >= THUB_MODULE_PATH_BYTES)
				path_len = THUB_MODULE_PATH_BYTES - 1;
			entry.path_len = path_len;
			memcpy(entry.path, path, path_len);
			entry.path[path_len] = '\0';
			if (copy_to_user(&entries[returned], &entry, sizeof(entry))) {
				rc = -EFAULT;
				break;
			}
			returned++;
		}
		mmap_read_unlock(mm);
	}

	kfree(path_buf);
	mmput(mm);

	req->returned = returned;
	req->total_matches = total_matches;
	return rc;
}

int thub_kernel_read_target(struct thub_session *session,
			    const struct thub_submit_read_request *req,
			    struct thub_range_result *range_results,
			    size_t max_results,
			    size_t *payload_bytes,
			    __u32 *status_out)
{
	struct task_struct *task;
	struct mm_struct *mm;
	size_t offset = 0;
	size_t i;

	if (req->target_slot >= THUB_MAX_TARGETS || !session->slots[req->target_slot].in_use) {
		*status_out = THUB_STATUS_BAD_TARGET_SLOT;
		return 0;
	}
	if (req->range_count == 0 || req->range_count > THUB_MAX_RANGES_PER_JOB ||
	    req->range_count > max_results) {
		*status_out = THUB_STATUS_BAD_REQUEST;
		return 0;
	}

	task = thub_lookup_task(&session->slots[req->target_slot].selector);
	if (IS_ERR(task)) {
		*status_out = THUB_STATUS_START_TIME_MISMATCH;
		return 0;
	}
	if (!task) {
		*status_out = THUB_STATUS_NO_TASK;
		return 0;
	}
	mm = get_task_mm(task);
	if (!mm) {
		put_task_struct(task);
		*status_out = THUB_STATUS_NO_MM;
		return 0;
	}

	*status_out = THUB_STATUS_OK;
	for (i = 0; i < req->range_count; i++) {
		const struct thub_read_range *range = &req->ranges[i];
		int bytes;

		range_results[i].remote_addr = range->remote_addr;
		range_results[i].requested_len = range->len;
		range_results[i].bytes_read = 0;
		range_results[i].status = THUB_RANGE_STATUS_OK;

		if (!range->len || offset + range->len > session->scratch_len) {
			range_results[i].status = THUB_RANGE_STATUS_FAULT;
			*status_out = THUB_STATUS_BAD_REQUEST;
			mmput(mm);
			put_task_struct(task);
			return 0;
		}

		bytes = access_process_vm(task, range->remote_addr,
					  session->scratch + offset, range->len, 0);
		if (bytes < 0)
			bytes = 0;
		range_results[i].bytes_read = bytes;

		if (bytes != range->len) {
			range_results[i].status = bytes > 0 ? THUB_RANGE_STATUS_PARTIAL : THUB_RANGE_STATUS_FAULT;
			*status_out = THUB_STATUS_PARTIAL_READ;
		}

		offset += bytes;
	}

	mmput(mm);
	put_task_struct(task);
	*payload_bytes = offset;
	return 0;
}

int thub_kernel_advance_tail(struct thub_session *session,
			     const struct thub_advance_tail_request *req)
{
	size_t capacity;
	size_t current_tail;
	size_t producer_head;
	size_t occupied;
	size_t advance;
	size_t requested_tail;

	if (!session || !req)
		return -EINVAL;

	capacity = session->ring_capacity_bytes;
	current_tail = session->consumer_tail;
	producer_head = session->producer_head;
	requested_tail = req->consumer_tail;

	if (!capacity)
		return -EINVAL;
	if (requested_tail >= capacity)
		return -EINVAL;
	if (requested_tail == current_tail)
		return 0;

	occupied = producer_head >= current_tail ?
		   producer_head - current_tail :
		   capacity - (current_tail - producer_head);
	advance = requested_tail >= current_tail ?
		  requested_tail - current_tail :
		  capacity - (current_tail - requested_tail);
	if (advance > occupied)
		return -EINVAL;

	session->consumer_tail = requested_tail;
	if (session->layout)
		session->layout->consumer_tail = session->consumer_tail;
	return 0;
}
