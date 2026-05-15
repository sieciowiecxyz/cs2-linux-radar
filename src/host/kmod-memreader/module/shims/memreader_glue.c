// SPDX-License-Identifier: GPL-2.0
#include <linux/fs.h>
#include <linux/init.h>
#include <linux/kernel.h>
#include <linux/kref.h>
#include <linux/miscdevice.h>
#include <linux/mm.h>
#include <linux/module.h>
#include <linux/poll.h>
#include <linux/slab.h>
#include <linux/uaccess.h>
#include <linux/vmalloc.h>

#include "../include/memreader_shims.h"

#define THUB_UAPI_VERSION 3
#define THUB_IOCTL_MAGIC 'T'
#define THUB_IOCTL_GET_INFO _IOR(THUB_IOCTL_MAGIC, 0x01, struct thub_get_info_response)
#define THUB_IOCTL_UPSERT_TARGET _IOW(THUB_IOCTL_MAGIC, 0x02, struct thub_upsert_target_request)
#define THUB_IOCTL_REMOVE_TARGET _IOW(THUB_IOCTL_MAGIC, 0x03, struct thub_remove_target_request)
#define THUB_IOCTL_SUBMIT_READ _IOW(THUB_IOCTL_MAGIC, 0x04, struct thub_submit_read_request)
#define THUB_IOCTL_ADVANCE_TAIL _IOW(THUB_IOCTL_MAGIC, 0x05, struct thub_advance_tail_request)
#define THUB_IOCTL_RESOLVE_HOST_PROCESS _IOWR(THUB_IOCTL_MAGIC, 0x06, struct thub_resolve_host_process_request)
#define THUB_IOCTL_INSPECT_HOST_PROCESS _IOWR(THUB_IOCTL_MAGIC, 0x07, struct thub_inspect_host_process_request)
#define THUB_IOCTL_LIST_MODULES _IOWR(THUB_IOCTL_MAGIC, 0x08, struct thub_list_modules_request)

static unsigned int thub_ring_bytes = THUB_DEFAULT_RING_BYTES;
module_param_named(ring_bytes, thub_ring_bytes, uint, 0600);
MODULE_PARM_DESC(ring_bytes, "Per-open ring mapping size for memreader");

struct memreader_session_ref {
	struct kref refs;
	struct thub_session session;
};

static inline struct memreader_session_ref *
memreader_session_ref_from_session(struct thub_session *session)
{
	return container_of(session, struct memreader_session_ref, session);
}

static inline struct thub_session *memreader_session_from_file(struct file *file)
{
	struct memreader_session_ref *session_ref = file->private_data;

	return session_ref ? &session_ref->session : NULL;
}

static void memreader_session_release_kref(struct kref *kref)
{
	struct memreader_session_ref *session_ref = container_of(kref, struct memreader_session_ref, refs);
	struct thub_session *session = &session_ref->session;

	vfree(session->scratch);
	vfree(session->mapping);
	kfree(session_ref);
}

static void memreader_session_get(struct memreader_session_ref *session_ref)
{
	kref_get(&session_ref->refs);
}

static void memreader_session_put(struct memreader_session_ref *session_ref)
{
	kref_put(&session_ref->refs, memreader_session_release_kref);
}

static void memreader_vma_open(struct vm_area_struct *vma)
{
	struct memreader_session_ref *session_ref = vma->vm_private_data;

	if (!session_ref)
		return;
	memreader_session_get(session_ref);
}

static void memreader_vma_close(struct vm_area_struct *vma)
{
	struct memreader_session_ref *session_ref = vma->vm_private_data;

	if (!session_ref)
		return;
	vma->vm_private_data = NULL;
	memreader_session_put(session_ref);
}

static const struct vm_operations_struct memreader_vm_ops = {
	.open = memreader_vma_open,
	.close = memreader_vma_close,
};

static int memreader_open(struct inode *inode, struct file *file)
{
	struct thub_session *session;
	struct memreader_session_ref *session_ref;
	size_t mapping_bytes;

	session_ref = kzalloc(sizeof(*session_ref), GFP_KERNEL);
	if (!session_ref)
		return -ENOMEM;
	kref_init(&session_ref->refs);
	session = &session_ref->session;

	mapping_bytes = max_t(size_t, PAGE_ALIGN(thub_ring_bytes), PAGE_SIZE * 4);
	session->mapping_bytes = mapping_bytes;
	session->mapping = vmalloc_user(mapping_bytes);
	if (!session->mapping) {
		kfree(session_ref);
		return -ENOMEM;
	}

	session->layout = session->mapping;
	session->scratch_len = THUB_MAX_PAYLOAD_BYTES;
	session->scratch = vzalloc(session->scratch_len);
	if (!session->scratch) {
		vfree(session->mapping);
		kfree(session_ref);
		return -ENOMEM;
	}

	init_waitqueue_head(&session->waitq);
	if (thub_rust_session_init(session, mapping_bytes)) {
		memreader_session_put(session_ref);
		return -EINVAL;
	}

	file->private_data = session_ref;
	return 0;
}

static int memreader_release(struct inode *inode, struct file *file)
{
	struct memreader_session_ref *session_ref = file->private_data;

	if (!session_ref)
		return 0;

	file->private_data = NULL;
	memreader_session_put(session_ref);
	return 0;
}

static __poll_t memreader_poll(struct file *file, poll_table *wait)
{
	struct thub_session *session = memreader_session_from_file(file);

	if (!session)
		return EPOLLERR;
	poll_wait(file, &session->waitq, wait);
	if (session->producer_head != session->consumer_tail)
		return EPOLLIN | EPOLLRDNORM;
	return 0;
}

static int memreader_mmap(struct file *file, struct vm_area_struct *vma)
{
	struct memreader_session_ref *session_ref = file->private_data;
	struct thub_session *session = session_ref ? &session_ref->session : NULL;
	size_t requested = vma->vm_end - vma->vm_start;
	int rc;

	if (!session || !session->mapping)
		return -EINVAL;
	if (vma->vm_pgoff != 0)
		return -EINVAL;
	if (vma->vm_flags & VM_WRITE)
		return -EPERM;
	if (requested > session->mapping_bytes)
		return -EINVAL;
	vm_flags_set(vma, VM_DONTDUMP | VM_DONTEXPAND);
	rc = remap_vmalloc_range(vma, session->mapping, 0);
	if (rc)
		return rc;
	vma->vm_ops = &memreader_vm_ops;
	vma->vm_private_data = session_ref;
	memreader_vma_open(vma);
	return 0;
}

static long memreader_ioctl(struct file *file, unsigned int cmd, unsigned long arg)
{
	struct thub_session *session = memreader_session_from_file(file);

	if (!session)
		return -EINVAL;

	switch (cmd) {
	case THUB_IOCTL_GET_INFO: {
		struct thub_get_info_response info = {
			.uapi_version = THUB_UAPI_VERSION,
			.ring_mapping_bytes = session->mapping_bytes,
			.max_targets = THUB_MAX_TARGETS,
			.max_ranges_per_job = THUB_MAX_RANGES_PER_JOB,
			.max_payload_bytes = THUB_MAX_PAYLOAD_BYTES,
		};

		if (copy_to_user((void __user *)arg, &info, sizeof(info)))
			return -EFAULT;
		return 0;
	}
	case THUB_IOCTL_UPSERT_TARGET: {
		struct thub_upsert_target_request req;

		if (copy_from_user(&req, (void __user *)arg, sizeof(req)))
			return -EFAULT;
		return thub_rust_upsert_target(session, &req);
	}
	case THUB_IOCTL_REMOVE_TARGET: {
		struct thub_remove_target_request req;

		if (copy_from_user(&req, (void __user *)arg, sizeof(req)))
			return -EFAULT;
		return thub_rust_remove_target(session, &req);
	}
	case THUB_IOCTL_SUBMIT_READ: {
		struct thub_submit_read_request req;
		struct thub_range_result range_results[THUB_MAX_RANGES_PER_JOB] = { 0 };
		size_t payload_bytes = 0;
		__u32 status = THUB_STATUS_INTERNAL;
		int rc;

		if (copy_from_user(&req, (void __user *)arg, sizeof(req)))
			return -EFAULT;

		rc = thub_kernel_read_target(session, &req, range_results,
					     THUB_MAX_RANGES_PER_JOB, &payload_bytes, &status);
		if (rc)
			return rc;
		rc = thub_rust_publish_record(session, &req, status, req.range_count,
					      range_results, session->scratch, payload_bytes);
		if (rc < 0)
			pr_warn_ratelimited("memreader: publish failed rc=%d status=%u slot=%u range_count=%u payload_bytes=%zu cap=%zu head=%llu tail=%llu dropped=%llu next_seq=%llu\n",
					    rc, status, req.target_slot, req.range_count, payload_bytes,
					    session->ring_capacity_bytes,
					    session->producer_head, session->consumer_tail,
					    session->dropped_records, session->next_seq);
		return rc;
	}
	case THUB_IOCTL_ADVANCE_TAIL: {
		struct thub_advance_tail_request req;

		if (copy_from_user(&req, (void __user *)arg, sizeof(req)))
			return -EFAULT;
		return thub_kernel_advance_tail(session, &req);
	}
	case THUB_IOCTL_RESOLVE_HOST_PROCESS: {
		struct thub_resolve_host_process_request req;
		int rc;

		if (copy_from_user(&req, (void __user *)arg, sizeof(req)))
			return -EFAULT;
		rc = thub_kernel_resolve_host_process(&req);
		if (rc)
			return rc;
		if (copy_to_user((void __user *)arg, &req, sizeof(req)))
			return -EFAULT;
		return 0;
	}
	case THUB_IOCTL_INSPECT_HOST_PROCESS: {
		struct thub_inspect_host_process_request req;
		int rc;

		if (copy_from_user(&req, (void __user *)arg, sizeof(req)))
			return -EFAULT;
		rc = thub_kernel_inspect_host_process(&req);
		if (rc)
			return rc;
		if (copy_to_user((void __user *)arg, &req, sizeof(req)))
			return -EFAULT;
		return 0;
	}
	case THUB_IOCTL_LIST_MODULES: {
		struct thub_list_modules_request req;
		int rc;

		if (copy_from_user(&req, (void __user *)arg, sizeof(req)))
			return -EFAULT;
		rc = thub_kernel_list_modules(&req);
		if (rc)
			return rc;
		if (copy_to_user((void __user *)arg, &req, sizeof(req)))
			return -EFAULT;
		return 0;
	}
	default:
		return -ENOTTY;
	}
}

static const struct file_operations memreader_fops = {
	.owner = THIS_MODULE,
	.open = memreader_open,
	.release = memreader_release,
	.poll = memreader_poll,
	.unlocked_ioctl = memreader_ioctl,
	.mmap = memreader_mmap,
	.llseek = noop_llseek,
};

static struct miscdevice memreader_miscdev = {
	.minor = MISC_DYNAMIC_MINOR,
	.name = "memreader",
	.fops = &memreader_fops,
	.mode = 0666,
};

static int __init memreader_init(void)
{
	pr_info("memreader: init\n");
	return misc_register(&memreader_miscdev);
}

static void __exit memreader_exit(void)
{
	misc_deregister(&memreader_miscdev);
	pr_info("memreader: exit\n");
}

void thub_kernel_wake_consumer(struct thub_session *session)
{
	wake_up_interruptible(&session->waitq);
}

module_init(memreader_init);
module_exit(memreader_exit);

MODULE_LICENSE("GPL");
MODULE_AUTHOR("OpenAI");
MODULE_DESCRIPTION("Memreader kernel module for low-overhead process memory snapshots");
