import re
import sys
import os
import pprint
import pandas as pd

pp = pprint.PrettyPrinter(depth=4)

def process_oha(file_path):
    with open(file_path, 'r') as file:
        content = file.read()

    # Regular expression pattern to find the response time histogram values
    pattern = r'(\d+\.\d+)\s\[(\d+)\]\s*\|'
    
    histogram_values = re.findall(pattern, content)
    
   # Calculate the median time from the histogram
    times = [float(time) for time, _ in histogram_values]
    counts = [int(count) for _, count in histogram_values]
    total_counts = sum(counts)
    median_index = total_counts // 2
    cumulative_count = 0
    median_time = None

    for time, count in histogram_values:
       cumulative_count += int(count)
       if cumulative_count >= median_index:
          median_time = float(time)
          break


    slowest_pattern = r'Slowest:\s+(\d+\.\d+)\ssecs'
    fastest_pattern = r'Fastest:\s+(\d+\.\d+)\ssecs'
    requests_per_sec_pattern = r'Requests\/sec:\s+(\d+\.\d+)'
    total_data_pattern = r'Total data:\s+(\d+\.\d+)\s([KMGTP]?i?B)'
    response_time_25_pattern = r'25%\sin\s(\d+\.\d+)\ssecs'
    response_time_50_pattern = r'50%\sin\s(\d+\.\d+)\ssecs'
    response_time_75_pattern = r'75%\sin\s(\d+\.\d+)\ssecs'
    response_time_90_pattern = r'90%\sin\s(\d+\.\d+)\ssecs'
    response_time_99_pattern = r'99%\sin\s(\d+\.\d+)\ssecs'

    slowest = float(re.search(slowest_pattern, content).group(1))
    fastest = float(re.search(fastest_pattern, content).group(1))
    requests_per_sec = float(re.search(requests_per_sec_pattern, content).group(1))
    total_data, data_unit = re.search(total_data_pattern, content).groups()
    response_time_25 = float(re.search(response_time_25_pattern, content).group(1))
    response_time_50 = float(re.search(response_time_50_pattern, content).group(1))
    response_time_75 = float(re.search(response_time_75_pattern, content).group(1))
    response_time_90 = float(re.search(response_time_90_pattern, content).group(1))
    response_time_99 = float(re.search(response_time_99_pattern, content).group(1))

    total_data = float(total_data)
    if data_unit.lower() == 'kib':
        total_data *= 1024
    elif data_unit.lower() == 'mib':
        total_data *= 1024 ** 2
    elif data_unit.lower() == 'gib':
        total_data *= 1024 ** 3
    elif data_unit.lower() == 'tib':
        total_data *= 1024 ** 4
    elif data_unit.lower() == 'pib':
        total_data *= 1024 ** 5

    file_name = os.path.basename(file_path)
    print(f"File: {file_path}")
    print(f"Median: {int(response_time_50 * 1000)}")
    print(f"Fastest: {int(fastest * 1000)}")
    print(f"Slowest: {int(slowest * 1000)}")
    print(f"Response time 25%: {int(response_time_25 * 1000)}")
    print(f"Response time 75%: {int(response_time_75 * 1000)}")
    print(f"Requests/sec: {float(requests_per_sec)}")
    print(f"Total data: {int(total_data)}")
    print(f"Total requests: {int(total_counts)}")
    print()

    return {
        "Slowest": int(slowest * 1000),
        "99th": int(response_time_99 * 1000),
        "90th": int(response_time_90 * 1000),
        "75th": int(response_time_75 * 1000),
        "50th": int(response_time_50 * 1000),
        "25th": int(response_time_25 * 1000),
        "Fastest": int(fastest * 1000),
        "Requests/sec": int(requests_per_sec),
        "Total data": int(total_data),
        "Total requests": int(total_counts),
    }

def process_warp(file_path):
    with open(file_path, 'r') as file:
        content = file.read()

    # Regular expression patterns to find the required fields
    requests_considered_pattern = r'Requests considered:\s+(\d+)'
    ttfb_line_pattern = r'TTFB:.+'
    ttfb_median_pattern = r'Median: (\d+\.?\d*)\s*(s|ms)'
    worst_pattern = r'Worst: (\d+\.?\d*)\s*(s|ms)'
    ttfb_99th_pattern = r'99th: (\d+\.?\d*)\s*(s|ms)'
    ttfb_90th_pattern = r'90th: (\d+\.?\d*)\s*(s|ms)'
    ttfb_75th_pattern = r'75th: (\d+\.?\d*)\s*(s|ms)'
    ttfb_25th_pattern = r'25th: (\d+\.?\d*)\s*(s|ms)'
    best_pattern = r'Best: (\d+\.?\d*)\s*(s|ms)'
    throughput_pattern = r'Average:\s+(\d+\.\d+)\sMiB/s,\s+(\d+\.\d+)\sobj/s'

    requests_considered = int(re.search(requests_considered_pattern, content).group(1))

    ttfb_lines = re.findall(ttfb_line_pattern, content)

    def convert_to_ms(match):
        value, unit = match.groups()
        if unit == 's':
            return float(value) * 1000
        else:
            return float(value)

    ttfb_median = convert_to_ms(re.search(ttfb_median_pattern, ttfb_lines[0]))
    best = convert_to_ms(re.search(best_pattern, ttfb_lines[0]))
    worst = convert_to_ms(re.search(worst_pattern, ttfb_lines[0]))
    ttfb_25th = convert_to_ms(re.search(ttfb_25th_pattern, ttfb_lines[0]))
    ttfb_75th = convert_to_ms(re.search(ttfb_75th_pattern, ttfb_lines[0]))
    ttfb_90th = convert_to_ms(re.search(ttfb_90th_pattern, ttfb_lines[0]))
    ttfb_99th = convert_to_ms(re.search(ttfb_99th_pattern, ttfb_lines[0]))

    throughput_mib_s, throughput_obj_s = re.search(throughput_pattern, content).groups()
    throughput_mib_s = float(throughput_mib_s)
    throughput_obj_s = float(throughput_obj_s)

    print(f"File: {file_path}")
    print(f"Requests considered: {requests_considered}")
    print(f"Median: {ttfb_median:.2f} ms")
    print(f"Best: {best:.2f} ms")
    print(f"Worst: {worst:.2f} ms")
    print(f"99th: {worst:.2f} ms")
    print(f"25th: {ttfb_25th:.2f} ms")
    print(f"75th: {ttfb_75th:.2f} ms")
    print(f"Throughput: {throughput_mib_s:.2f} MiB/s, {throughput_obj_s:.2f} obj/s")
    print()

    return {
        "Total requests": requests_considered,
        "Slowest": worst,
        "99th": ttfb_99th,
        "90th": ttfb_90th,
        "75th": ttfb_75th,
        "Median": ttfb_median,
        "25th": ttfb_25th,
        "Fastest": best,
        "Throughput": int(throughput_obj_s),
    }

def process_iftop(file_path):
    with open(file_path, 'r') as file:
        content = file.read()

    # Regular expression pattern to find the cumulative values section
    cumulative_pattern = r'Cumulative \(sent/received/total\):\s*([\d.]+[KMG]?B)\s+([\d.]+[KMG]?B)\s+([\d.]+[KMG]?B)'

    # Find the cumulative values from the matching line
    cumulative_values_match = re.findall(cumulative_pattern, content)[-1]

    sent_cumulative, received_cumulative, total_cumulative = cumulative_values_match

    print(f"File: {file_path}")
    print(f"Cumulative Sent: {sent_cumulative}")
    print(f"Cumulative Received: {received_cumulative}")
    print(f"Cumulative Total: {total_cumulative}")
    print()
    
    sent_cumulative = get_data_size_in_bytes(sent_cumulative)
    received_cumulative = get_data_size_in_bytes(received_cumulative)
    total_cumulative = get_data_size_in_bytes(total_cumulative)

    return {
        "Cumulative Sent": sent_cumulative,
        "Cumulative Received": received_cumulative,
        "Cumulative Total": total_cumulative,
    }


def parse_test_size_suffix(file_name):
    # Regular expression pattern to find the size suffix (e.g., "1KiB", "10KiB", "1MiB")
    size_pattern = r'([\d.]+)([KMGTP]iB)'

    match = re.search(size_pattern, file_name)
    if match:
        size_value, size_unit = match.groups()
        size_value = float(size_value)
        size_unit = size_unit.lower()

        return size_value, size_unit
    return 0, "b"

def get_test_size_in_bytes(file_name):
    size_value, size_unit = parse_test_size_suffix(file_name)
    size_value = float(size_value)
    size_unit = size_unit.lower()

    if size_unit == 'kib':
        return int(size_value * 1024)
    elif size_unit == 'mib':
        return int(size_value * 1024 ** 2)
    elif size_unit == 'gib':
        return int(size_value * 1024 ** 3)
    elif size_unit == 'tib':
        return int(size_value * 1024 ** 4)
    elif size_unit == 'pib':
        return int(size_value * 1024 ** 5)

    # If no size suffix found or unsupported unit, return 0
    return 0

def get_data_size_in_bytes(file_name):
    # Regular expression pattern to find the size suffix (e.g., "1KB", "10KB", "1MB")
    size_pattern = r'([\d.]+)([KMGTP]B)'

    match = re.search(size_pattern, file_name)
    if match:
        size_value, size_unit = match.groups()
        size_value = float(size_value)
        size_unit = size_unit.lower()

        if size_unit == 'kb':
            return int(size_value * 1024)
        elif size_unit == 'mb':
            return int(size_value * 1024 ** 2)
        elif size_unit == 'gb':
            return int(size_value * 1024 ** 3)
        elif size_unit == 'tb':
            return int(size_value * 1024 ** 4)
        elif size_unit == 'pb':
            return int(size_value * 1024 ** 5)

        

    # If no size suffix found or unsupported unit, return 0
    return 0

def get_file_type(file_name):
    if file_name.startswith("warp"):
        return 1, "warp"
    if file_name.startswith("iftop-warp"):
        return 2, "iftop-warp"
    if file_name.startswith("oha"):
        return 3, "oha"
    if file_name.startswith("iftop-oha"):
        return 4, "iftop-oha"
    return 0, ""

def write_to_spreadsheet(data_dict, output_file):
    # Create an empty DataFrame to store the data
    df = pd.DataFrame()

    # Flatten the nested dictionary and add the values to the DataFrame
    for tool, sizes in data_dict.items():
        for size, values in sizes.items():
            for metric, value in values.items():
                row_name = f"{tool} {metric}"
                column_name = f"{size}"
                df.loc[row_name, column_name] = value

    # Insert an empty column to the right
    df[""] = ""

    # Add the "Units" column with the specified values
    units = [
        "#",
        "ms",
        "ms",
        "ms",
        "ms",
        "ms",
        "ms",
        "ms",
        "ms",
        "obj/s",
        "B",
        "B",
        "B",
        "ms",
        "ms",
        "ms",
        "ms",
        "ms",
        "ms",
        "req/s",
        "B",
        "B",
        "B",
        "B",
        "#",
        "B",
        "B",
    ]
    df["Units"] = units

    # Write the DataFrame to a spreadsheet
    df.to_excel(output_file)

def parse_files_in_directory(directory_path):
    # Get a list of files in the directory
    file_list = os.listdir(directory_path)

    # Sort the files based on the indicated size in ascending order
    file_list.sort(key=lambda file_name: (get_test_size_in_bytes(file_name), get_file_type(file_name)[0]) )

    results = {}

    for file in file_list:
        file_path = os.path.join(directory_path, file)
        
        if not os.path.isfile(file_path):
            continue

        size, unit = parse_test_size_suffix(file)
        test_size = f"{int(size)}{unit}"
        test_type = get_file_type(file)[1]

        if file.startswith("warp"):
            r = process_warp(file_path)
        if file.startswith("oha"):
            r = process_oha(file_path)
        if file.startswith("iftop"):
            r = process_iftop(file_path)

        results[test_type] = results.get(test_type, {})
        results[test_type][test_size] = r

    results["calc"] = {}

    sizes = next(iter(results.values())).keys()

    for size in sizes:
        warp_req = results["warp"][size]["Total requests"]
        oha_req = results["oha"][size]["Total requests"]

        warp_data = results["iftop-warp"][size]["Cumulative Total"]
        oha_data = results["iftop-oha"][size]["Cumulative Total"]

        results["calc"][size] = {}
        results["calc"][size]["warp Avg B/Req"] = warp_data // warp_req
        results["calc"][size]["oha Avg B/Req"] = oha_data // oha_req

    # pp.pprint(results)
    test_name = os.path.basename(directory_path)
    write_to_spreadsheet(results, f"{test_name}-output.ods")
    

if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Usage: python script.py <directory_path>")
    else:
        directory_path = sys.argv[1]
        parse_files_in_directory(directory_path)
