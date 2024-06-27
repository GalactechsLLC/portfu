<script>
    import { onMount } from 'svelte';
    import { createEventDispatcher } from 'svelte';

    const dispatch = createEventDispatcher();

    let userProfile = {
        name: '',
        email: '',
        password: '',
        profilePicture: '',
        bio: '',
        phoneNumber: '',
        address: '',
        dateOfBirth: '',
        gender: '',
        preferences: {
            newsletter: false,
            notifications: false
        }
    };

    onMount(async () => {
        const response = await fetch('$lib/assets/data/userProfile.json');
        userProfile = await response.json();
    });

    function handleInputChange(event) {
        const { name, value, type, checked } = event.target;
        if (type === 'checkbox') {
            userProfile.preferences = { ...userProfile.preferences, [name]: checked };
        } else {
            userProfile = { ...userProfile, [name]: value };
        }
    }

    function handleFileChange(event) {
        const file = event.target.files[0];
        if (file) {
            const reader = new FileReader();
            reader.onload = (e) => {
                userProfile = { ...userProfile, profilePicture: e.target.result };
            };
            reader.readAsDataURL(file);
        }
    }

    function saveProfile() {
        if (userProfile.password && userProfile.password !== userProfile.confirmPassword) {
            alert('Passwords do not match');
            return;
        }
        // Dispatch a save event with the user profile data
        dispatch('save', userProfile);
    }
</script>

<style>
    .profile-editor {
        max-width: 600px;
        margin: 0 auto;
    }
    .profile-picture {
        width: 100px;
        height: 100px;
        border-radius: 50%;
        object-fit: cover;
        margin-bottom: 10px;
    }
    input[type="file"] {
        display: none;
    }
    label {
        cursor: pointer;
        color: blue;
        text-decoration: underline;
    }
    .checkbox {
        display: flex;
        align-items: center;
    }
    .checkbox label {
        margin-left: 8px;
        cursor: pointer;
    }
</style>

<div class="profile-editor">
    <h1>Account Settings</h1>
    <div>
        <label for="profilePicture">
            <img
                    src={userProfile.profilePicture || 'https://via.placeholder.com/100'}
                    alt="Profile Picture"
                    class="profile-picture"
            />
        </label>
        <input
                type="file"
                id="profilePicture"
                accept="image/*"
                on:change={handleFileChange}
        />
    </div>
    <div>
        <label>Name:</label>
        <input
                type="text"
                name="name"
                bind:value={userProfile.name}
                on:input={handleInputChange}
        />
    </div>
    <div>
        <label>Email:</label>
        <input
                type="email"
                name="email"
                bind:value={userProfile.email}
                on:input={handleInputChange}
        />
    </div>
    <div>
        <label>Password:</label>
        <input
                type="password"
                name="password"
                bind:value={userProfile.password}
                on:input={handleInputChange}
        />
    </div>
    <div>
        <label>Confirm Password:</label>
        <input
                type="password"
                name="confirmPassword"
                bind:value={userProfile.confirmPassword}
                on:input={handleInputChange}
        />
    </div>
    <div>
        <label>Bio:</label>
        <textarea
                name="bio"
                bind:value={userProfile.bio}
                on:input={handleInputChange}
        ></textarea>
    </div>
    <div>
        <label>Phone Number:</label>
        <input
                type="text"
                name="phoneNumber"
                bind:value={userProfile.phoneNumber}
                on:input={handleInputChange}
        />
    </div>
    <div>
        <label>Address:</label>
        <input
                type="text"
                name="address"
                bind:value={userProfile.address}
                on:input={handleInputChange}
        />
    </div>
    <div>
        <label>Date of Birth:</label>
        <input
                type="date"
                name="dateOfBirth"
                bind:value={userProfile.dateOfBirth}
                on:input={handleInputChange}
        />
    </div>
    <div>
        <label>Gender:</label>
        <select name="gender" bind:value={userProfile.gender} on:input={handleInputChange}>
            <option value="Male">Male</option>
            <option value="Female">Female</option>
            <option value="Other">Other</option>
        </select>
    </div>
    <div class="checkbox">
        <input
                type="checkbox"
                id="newsletter"
                name="newsletter"
                checked={userProfile.preferences.newsletter}
                on:change={handleInputChange}
        />
        <label for="newsletter">Subscribe to Newsletter</label>
    </div>
    <div class="checkbox">
        <input
                type="checkbox"
                id="notifications"
                name="notifications"
                checked={userProfile.preferences.notifications}
                on:change={handleInputChange}
        />
        <label for="notifications">Enable Notifications</label>
    </div>
    <button on:click={saveProfile}>Save</button>
</div>
