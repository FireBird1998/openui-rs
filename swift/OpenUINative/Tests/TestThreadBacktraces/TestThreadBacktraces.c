#include "TestThreadBacktraces.h"

#include <dlfcn.h>
#include <mach/mach.h>
#include <mach/mach_vm.h>
#include <stdint.h>
#include <stdio.h>
#if defined(__arm64__)
#include <ptrauth.h>
#endif

struct Frame {
    uintptr_t previous;
    uintptr_t return_address;
};

static uintptr_t code_address(uintptr_t address) {
#if defined(__arm64__)
    return (uintptr_t)ptrauth_strip((void *)address, ptrauth_key_return_address);
#else
    return address;
#endif
}

static void write_frame(int fd, unsigned depth, uintptr_t address) {
    Dl_info info;
    if (dladdr((void *)address, &info)) {
        if (info.dli_sname) {
            dprintf(fd, "  %3u  %p %s + %lu\n", depth, (void *)address, info.dli_sname,
                    (unsigned long)(address - (uintptr_t)info.dli_saddr));
        } else {
            dprintf(fd, "  %3u  %p %s + %lu\n", depth, (void *)address, info.dli_fname,
                    (unsigned long)(address - (uintptr_t)info.dli_fbase));
        }
    } else {
        dprintf(fd, "  %3u  %p\n", depth, (void *)address);
    }
}

static void write_thread(int fd, thread_t thread) {
#if defined(__arm64__)
    arm_thread_state64_t state;
    mach_msg_type_number_t count = ARM_THREAD_STATE64_COUNT;
    kern_return_t result = thread_get_state(thread, ARM_THREAD_STATE64, (thread_state_t)&state, &count);
    uintptr_t pc = result == KERN_SUCCESS ? arm_thread_state64_get_pc(state) : 0;
    uintptr_t fp = result == KERN_SUCCESS ? arm_thread_state64_get_fp(state) : 0;
#elif defined(__x86_64__)
    x86_thread_state64_t state;
    mach_msg_type_number_t count = x86_THREAD_STATE64_COUNT;
    kern_return_t result = thread_get_state(thread, x86_THREAD_STATE64, (thread_state_t)&state, &count);
    uintptr_t pc = result == KERN_SUCCESS ? state.__rip : 0;
    uintptr_t fp = result == KERN_SUCCESS ? state.__rbp : 0;
#endif
    thread_identifier_info_data_t id;
    mach_msg_type_number_t id_count = THREAD_IDENTIFIER_INFO_COUNT;
    kern_return_t identified = thread_info(thread, THREAD_IDENTIFIER_INFO, (thread_info_t)&id, &id_count);
    thread_extended_info_data_t details;
    mach_msg_type_number_t details_count = THREAD_EXTENDED_INFO_COUNT;
    kern_return_t described = thread_info(thread, THREAD_EXTENDED_INFO, (thread_info_t)&details, &details_count);
    dprintf(fd, "\nThread %llu", identified == KERN_SUCCESS ? id.thread_id : (uint64_t)thread);
    if (described == KERN_SUCCESS && details.pth_name[0]) {
        dprintf(fd, " %.*s", (int)sizeof(details.pth_name), details.pth_name);
    }
    dprintf(fd, "\n");
    if (result != KERN_SUCCESS) {
        dprintf(fd, "  thread_get_state: %s\n", mach_error_string(result));
        return;
    }

    // Keep other threads running. A suspended thread could own a lock
    // needed to write or symbolize the report. Validate each frame read.
    write_frame(fd, 0, pc);
    for (unsigned depth = 1; depth < 256 && fp && fp % sizeof(uintptr_t) == 0; ++depth) {
        struct Frame frame;
        mach_vm_size_t bytes = 0;
        if (mach_vm_read_overwrite(mach_task_self(), fp, sizeof(frame), (mach_vm_address_t)&frame, &bytes) != KERN_SUCCESS ||
            bytes != sizeof(frame) || !frame.return_address) {
            break;
        }
        write_frame(fd, depth, code_address(frame.return_address) - 1);
        if (frame.previous <= fp) break;
        fp = frame.previous;
    }
}

void write_test_thread_backtraces(int fd) {
    thread_act_array_t threads;
    mach_msg_type_number_t count;
    kern_return_t result = task_threads(mach_task_self(), &threads, &count);
    if (result != KERN_SUCCESS) {
        dprintf(fd, "task_threads: %s\n", mach_error_string(result));
        return;
    }
    dprintf(fd, "In-process backtraces (%u threads; frame pointers)\n", count);
    for (mach_msg_type_number_t i = 0; i < count; ++i) {
        write_thread(fd, threads[i]);
        mach_port_deallocate(mach_task_self(), threads[i]);
    }
    vm_deallocate(mach_task_self(), (vm_address_t)threads, count * sizeof(*threads));
}
